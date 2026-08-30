use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use feilian_helper_client::{ClientError, HelperClient};
use feilian_ipc::{
    ActiveTunnel, HelperState, TransportProtocol, TunnelMode as IpcTunnelMode, TunnelSpec,
    TunnelStats, TunnelStatus,
};
use feilian_lite::{
    get_company_url, Client, Config, QrChallenge, QrPollStatus, VpnNode, WgConf, PLATFORM_LARK,
    PLATFORM_OIDC,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::launcher::HelperLauncher;
use crate::secret_store::{
    SecretStore, SystemSecretStore, ACCOUNT_PASSWORD, SOCKS5_PASSWORD, TOTP_SECRET,
    WIREGUARD_PRIVATE_KEY,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperMode {
    SystemSplit,
    Socks5,
}

#[derive(Clone, Debug, Serialize)]
pub struct HelperSnapshot {
    pub mode: HelperMode,
    pub reachable: bool,
    pub state: HelperState,
    pub active: Option<ActiveTunnel>,
    pub stats: TunnelStats,
    pub error: Option<ControllerError>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ControllerError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

pub struct AppController {
    endpoints: HelperEndpoints,
    launcher: HelperLauncher,
    auth: Mutex<AuthState>,
    profile_path: PathBuf,
    secret_store: Arc<dyn SecretStore>,
}

impl AppController {
    pub fn new(data_dir: impl AsRef<Path>) -> Self {
        Self::with_secret_store(data_dir, Arc::new(SystemSecretStore))
    }

    fn with_secret_store(data_dir: impl AsRef<Path>, secret_store: Arc<dyn SecretStore>) -> Self {
        let data_dir = data_dir.as_ref();
        Self {
            endpoints: HelperEndpoints::new(data_dir),
            launcher: HelperLauncher::new(helper_binary_path()),
            auth: Mutex::new(AuthState::default()),
            profile_path: data_dir.join("profile.json"),
            secret_store,
        }
    }

    pub async fn auth_status(&self) -> Result<AuthSnapshot, ControllerError> {
        let mut auth = self.auth.lock().await;
        self.load_auth_if_needed(&mut auth).await?;
        Ok(auth.snapshot())
    }

    pub async fn initialize(&self) -> Result<(), ControllerError> {
        let mut auth = self.auth.lock().await;
        self.load_auth_if_needed(&mut auth).await
    }

    pub async fn auth_configure(
        &self,
        configuration: AuthConfiguration,
    ) -> Result<AuthSnapshot, ControllerError> {
        let company_code = configuration.company_code.trim();
        if company_code.is_empty() {
            return Err(ControllerError::new(
                "invalid_company_code",
                "请输入企业代码",
                false,
            ));
        }

        let company = get_company_url(company_code).await.map_err(|error| {
            ControllerError::new("company_discovery_failed", error.to_string(), true)
        })?;
        let platform = configuration.platform.as_config_value().to_string();
        let config = Config::create_desktop_profile(
            &self.profile_path,
            company_code.to_string(),
            company.domain,
            platform,
        )
        .await
        .map_err(auth_storage_error)?;
        self.store_secret(
            WIREGUARD_PRIVATE_KEY,
            config.private_key.as_deref().ok_or_else(|| {
                ControllerError::new(
                    "private_key_missing",
                    "WireGuard private key was not generated",
                    false,
                )
            })?,
        )?;
        self.delete_optional_profile_secrets()?;
        let client = Client::new(config)
            .map_err(|error| ControllerError::new("auth_client_failed", error.to_string(), true))?;
        let display_name = if company.zh_name.is_empty() {
            company.en_name
        } else {
            company.zh_name
        };

        let mut auth = self.auth.lock().await;
        auth.loaded = true;
        auth.session = Some(AuthSession {
            client,
            company_code: company_code.to_string(),
            company_name: display_name,
            platform: configuration.platform,
            challenge: None,
            nodes: Vec::new(),
            active_connection: None,
        });
        Ok(auth.snapshot())
    }

    pub async fn auth_begin_qr(&self) -> Result<AuthSnapshot, ControllerError> {
        let mut auth = self.auth.lock().await;
        self.load_auth_if_needed(&mut auth).await?;
        let session = auth.session.as_mut().ok_or_else(not_configured_error)?;
        let challenge = session.client.begin_qr_login().await.map_err(|error| {
            ControllerError::new("qr_login_start_failed", error.to_string(), true)
        })?;
        session.challenge = Some(challenge);
        Ok(auth.snapshot())
    }

    pub async fn auth_poll_qr(&self) -> Result<AuthSnapshot, ControllerError> {
        let mut auth = self.auth.lock().await;
        self.load_auth_if_needed(&mut auth).await?;
        let session = auth.session.as_mut().ok_or_else(not_configured_error)?;
        let token = session
            .challenge
            .as_ref()
            .map(|challenge| challenge.token.clone())
            .ok_or_else(|| {
                ControllerError::new("qr_challenge_missing", "请重新获取登录二维码", true)
            })?;
        match session
            .client
            .poll_qr_login(&token)
            .await
            .map_err(|error| {
                ControllerError::new("qr_login_poll_failed", error.to_string(), true)
            })? {
            QrPollStatus::Pending => {}
            QrPollStatus::Authenticated => {
                if let Some(secret) = session.client.totp_secret() {
                    self.store_secret(TOTP_SECRET, secret)?;
                }
                session.challenge = None;
                session.nodes = load_nodes(&mut session.client).await?;
            }
        }
        Ok(auth.snapshot())
    }

    pub async fn auth_refresh_nodes(&self) -> Result<AuthSnapshot, ControllerError> {
        let mut auth = self.auth.lock().await;
        self.load_auth_if_needed(&mut auth).await?;
        let session = auth.session.as_mut().ok_or_else(not_configured_error)?;
        if session.client.need_login() {
            return Err(ControllerError::new(
                "authentication_required",
                "请先完成飞书或 OIDC 登录",
                true,
            ));
        }
        session.nodes = load_nodes(&mut session.client).await?;
        Ok(auth.snapshot())
    }

    pub async fn auth_reset(&self) -> Result<AuthSnapshot, ControllerError> {
        let mut auth = self.auth.lock().await;
        if auth
            .session
            .as_ref()
            .is_some_and(|session| session.active_connection.is_some())
        {
            return Err(ControllerError::new(
                "disconnect_required",
                "请先断开当前 VPN 连接",
                false,
            ));
        }
        if let Some(session) = auth.session.as_mut() {
            let _ = session.client.logout().await;
        }
        self.delete_all_profile_secrets()?;
        auth.loaded = true;
        auth.session = None;
        remove_if_exists(&self.profile_path).await?;
        remove_if_exists(
            &self
                .profile_path
                .with_file_name("feilian-lite_cookies.json"),
        )
        .await?;
        Ok(auth.snapshot())
    }

    pub async fn status(&self, mode: HelperMode) -> HelperSnapshot {
        let client = self.client(mode);
        match client.status().await {
            Ok(status) => {
                let stats = if status.state == HelperState::Running {
                    client.stats().await.unwrap_or_default()
                } else {
                    TunnelStats::default()
                };
                HelperSnapshot::from_status(mode, status, stats)
            }
            Err(error) => HelperSnapshot::unavailable(mode, error),
        }
    }

    pub async fn connect(
        &self,
        mode: HelperMode,
        node_id: i32,
    ) -> Result<HelperSnapshot, ControllerError> {
        let mut auth = self.auth.lock().await;
        self.load_auth_if_needed(&mut auth).await?;
        let session = auth.session.as_mut().ok_or_else(not_configured_error)?;
        if session.client.need_login() {
            return Err(ControllerError::new(
                "authentication_required",
                "请先完成飞书或 OIDC 登录",
                true,
            ));
        }
        if session.active_connection.is_some() {
            return Err(ControllerError::new(
                "already_connected",
                "当前已有活动连接",
                false,
            ));
        }
        let node_name = session
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .map(|node| node.name.clone())
            .ok_or_else(|| ControllerError::new("node_not_found", "所选节点不可用", true))?;

        let helper = self.launcher.ensure_running(mode, &self.endpoints).await?;
        let wg_conf = session
            .client
            .connect_vpn_node(node_id)
            .await
            .map_err(|error| {
                ControllerError::new("tunnel_prepare_failed", error.to_string(), true)
            })?;
        let spec = tunnel_spec(mode, node_name, &wg_conf);
        match helper.start(spec).await {
            Ok(status) => {
                session.active_connection = Some(ActiveConnection {
                    mode,
                    config: wg_conf,
                });
                Ok(HelperSnapshot::from_status(
                    mode,
                    status,
                    TunnelStats::default(),
                ))
            }
            Err(error) => {
                let _ = session.client.disconnect_vpn(&wg_conf).await;
                Err(ControllerError::from_helper_client(error))
            }
        }
    }

    pub async fn stop(&self, mode: HelperMode) -> HelperSnapshot {
        let client = self.client(mode);
        match client.stop().await {
            Ok(status) => {
                let mut snapshot =
                    HelperSnapshot::from_status(mode, status, TunnelStats::default());
                if let Err(error) = self.release_server_connection(mode).await {
                    snapshot.error = Some(error);
                }
                snapshot
            }
            Err(error) => HelperSnapshot::unavailable(mode, error),
        }
    }

    pub async fn cleanup(&self, mode: HelperMode) -> HelperSnapshot {
        let client = self.client(mode);
        match client.cleanup().await {
            Ok(status) => {
                let mut snapshot =
                    HelperSnapshot::from_status(mode, status, TunnelStats::default());
                if let Err(error) = self.release_server_connection(mode).await {
                    snapshot.error = Some(error);
                }
                snapshot
            }
            Err(error) => HelperSnapshot::unavailable(mode, error),
        }
    }

    fn client(&self, mode: HelperMode) -> HelperClient {
        self.endpoints.client(mode)
    }

    async fn release_server_connection(&self, mode: HelperMode) -> Result<(), ControllerError> {
        let mut auth = self.auth.lock().await;
        let Some(session) = auth.session.as_mut() else {
            return Ok(());
        };
        let Some(active) = session.active_connection.take() else {
            return Ok(());
        };
        if active.mode != mode {
            session.active_connection = Some(active);
            return Ok(());
        }
        session
            .client
            .disconnect_vpn(&active.config)
            .await
            .map_err(|error| {
                ControllerError::new("server_disconnect_failed", error.to_string(), true)
            })
    }

    async fn load_auth_if_needed(&self, auth: &mut AuthState) -> Result<(), ControllerError> {
        if auth.loaded {
            return Ok(());
        }
        if !self.profile_path.exists() {
            auth.loaded = true;
            return Ok(());
        }

        let profile = self.profile_path.to_string_lossy().into_owned();
        let mut config = Config::from_desktop_profile(&profile)
            .await
            .map_err(auth_storage_error)?;
        self.restore_or_migrate_secrets(&mut config)?;
        config.save().await.map_err(auth_storage_error)?;
        let company_code = config.company_name.clone();
        let platform = AuthPlatform::from_config_value(config.platform.as_deref());
        let client = Client::new(config)
            .map_err(|error| ControllerError::new("auth_client_failed", error.to_string(), true))?;
        auth.session = Some(AuthSession {
            client,
            company_name: company_code.clone(),
            company_code,
            platform,
            challenge: None,
            nodes: Vec::new(),
            active_connection: None,
        });
        auth.loaded = true;
        Ok(())
    }

    fn restore_or_migrate_secrets(&self, config: &mut Config) -> Result<(), ControllerError> {
        config.private_key = Some(self.restore_or_migrate_required(
            WIREGUARD_PRIVATE_KEY,
            config.private_key.take(),
            "WireGuard private key is missing from secure storage; reconfigure the enterprise",
        )?);
        config.code = self.restore_or_migrate_optional(TOTP_SECRET, config.code.take())?;
        config.password =
            self.restore_or_migrate_optional(ACCOUNT_PASSWORD, config.password.take())?;
        config.socks5_password =
            self.restore_or_migrate_optional(SOCKS5_PASSWORD, config.socks5_password.take())?;
        config.keep_secrets_out_of_profile();
        Ok(())
    }

    fn restore_or_migrate_required(
        &self,
        name: &str,
        plaintext: Option<String>,
        missing_message: &str,
    ) -> Result<String, ControllerError> {
        if let Some(value) = plaintext {
            self.store_secret(name, &value)?;
            return Ok(value);
        }
        self.secret_store
            .get(name)
            .map_err(secret_storage_error)?
            .ok_or_else(|| {
                ControllerError::new("secret_missing", missing_message.to_string(), false)
            })
    }

    fn restore_or_migrate_optional(
        &self,
        name: &str,
        plaintext: Option<String>,
    ) -> Result<Option<String>, ControllerError> {
        if let Some(value) = plaintext {
            self.store_secret(name, &value)?;
            return Ok(Some(value));
        }
        self.secret_store.get(name).map_err(secret_storage_error)
    }

    fn store_secret(&self, name: &str, value: &str) -> Result<(), ControllerError> {
        self.secret_store
            .set(name, value)
            .map_err(secret_storage_error)
    }

    fn delete_optional_profile_secrets(&self) -> Result<(), ControllerError> {
        for name in [TOTP_SECRET, ACCOUNT_PASSWORD, SOCKS5_PASSWORD] {
            self.secret_store
                .delete(name)
                .map_err(secret_storage_error)?;
        }
        Ok(())
    }

    fn delete_all_profile_secrets(&self) -> Result<(), ControllerError> {
        self.delete_optional_profile_secrets()?;
        self.secret_store
            .delete(WIREGUARD_PRIVATE_KEY)
            .map_err(secret_storage_error)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthPlatform {
    Feishu,
    Oidc,
}

impl AuthPlatform {
    fn as_config_value(self) -> &'static str {
        match self {
            Self::Feishu => PLATFORM_LARK,
            Self::Oidc => PLATFORM_OIDC,
        }
    }

    fn from_config_value(value: Option<&str>) -> Self {
        match value {
            Some(PLATFORM_OIDC) => Self::Oidc,
            _ => Self::Feishu,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfiguration {
    pub company_code: String,
    pub platform: AuthPlatform,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthSnapshot {
    pub configured: bool,
    pub authenticated: bool,
    pub company_code: Option<String>,
    pub company_name: Option<String>,
    pub platform: Option<AuthPlatform>,
    pub challenge: Option<DesktopQrChallenge>,
    pub nodes: Vec<AuthNode>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DesktopQrChallenge {
    pub login_url: String,
    pub expires_at_unix: Option<u64>,
}

impl From<&QrChallenge> for DesktopQrChallenge {
    fn from(challenge: &QrChallenge) -> Self {
        Self {
            login_url: challenge.login_url.clone(),
            expires_at_unix: challenge.expires_at_unix,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AuthNode {
    pub id: i32,
    pub name: String,
    pub english_name: Option<String>,
    pub address: String,
    pub protocol: &'static str,
    pub latency_ms: Option<i64>,
    pub available: bool,
}

impl From<VpnNode> for AuthNode {
    fn from(node: VpnNode) -> Self {
        let name = if node.name.trim().is_empty() {
            node.en_name.clone()
        } else {
            node.name.trim().to_string()
        };
        let english_name = (!node.en_name.trim().is_empty() && node.en_name.trim() != name)
            .then(|| node.en_name.trim().to_string());
        let protocol = match node.protocol_mode {
            1 => "tcp",
            2 => "udp",
            _ => "unknown",
        };
        let host = if node.ip.contains(':') {
            format!("[{}]", node.ip)
        } else {
            node.ip
        };

        Self {
            id: node.id,
            name,
            english_name,
            address: format!("{host}:{}", node.vpn_port),
            protocol,
            latency_ms: node.latency_ms,
            available: node.latency_ms.is_some() && protocol != "unknown",
        }
    }
}

#[derive(Default)]
struct AuthState {
    loaded: bool,
    session: Option<AuthSession>,
}

impl AuthState {
    fn snapshot(&self) -> AuthSnapshot {
        match &self.session {
            Some(session) => AuthSnapshot {
                configured: true,
                authenticated: !session.client.need_login(),
                company_code: Some(session.company_code.clone()),
                company_name: Some(session.company_name.clone()),
                platform: Some(session.platform),
                challenge: session.challenge.as_ref().map(DesktopQrChallenge::from),
                nodes: session.nodes.clone(),
            },
            None => AuthSnapshot {
                configured: false,
                authenticated: false,
                company_code: None,
                company_name: None,
                platform: None,
                challenge: None,
                nodes: Vec::new(),
            },
        }
    }
}

struct AuthSession {
    client: Client,
    company_code: String,
    company_name: String,
    platform: AuthPlatform,
    challenge: Option<QrChallenge>,
    nodes: Vec<AuthNode>,
    active_connection: Option<ActiveConnection>,
}

struct ActiveConnection {
    mode: HelperMode,
    config: WgConf,
}

fn tunnel_spec(mode: HelperMode, node_name: String, config: &WgConf) -> TunnelSpec {
    TunnelSpec {
        node_name,
        interface_name: tunnel_interface_name(mode).to_string(),
        mode: match mode {
            HelperMode::SystemSplit => IpcTunnelMode::SystemSplit,
            HelperMode::Socks5 => IpcTunnelMode::Socks5 {
                listen_addr: "127.0.0.1:1080".to_string(),
                username: None,
                password: None,
            },
        },
        address: config.address.clone(),
        address6: config.address6.clone(),
        peer_address: config.peer_address.clone(),
        mtu: config.mtu,
        public_key: config.public_key.clone(),
        private_key: config.private_key.clone(),
        peer_key: config.peer_key.clone(),
        allowed_ips: config.allowed_ips.clone(),
        routes: if mode == HelperMode::SystemSplit {
            config.routes.clone()
        } else {
            Vec::new()
        },
        dns: config.dns.clone(),
        protocol: if config.protocol == 1 {
            TransportProtocol::Tcp
        } else {
            TransportProtocol::Udp
        },
    }
}

fn tunnel_interface_name(mode: HelperMode) -> &'static str {
    if cfg!(target_os = "macos") && mode == HelperMode::SystemSplit {
        "utun"
    } else {
        "feilian-lite"
    }
}

async fn load_nodes(client: &mut Client) -> Result<Vec<AuthNode>, ControllerError> {
    client
        .list_vpn_nodes()
        .await
        .map(|nodes| nodes.into_iter().map(AuthNode::from).collect())
        .map_err(|error| ControllerError::new("node_list_failed", error.to_string(), true))
}

async fn remove_if_exists(path: &Path) -> Result<(), ControllerError> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(auth_storage_error(error)),
    }
}

fn auth_storage_error(error: impl std::fmt::Display) -> ControllerError {
    ControllerError::new("auth_storage_failed", error.to_string(), true)
}

fn secret_storage_error(error: impl std::fmt::Display) -> ControllerError {
    ControllerError::new("secret_storage_failed", error.to_string(), true)
}

fn not_configured_error() -> ControllerError {
    ControllerError::new("auth_not_configured", "请先配置企业代码", false)
}

impl ControllerError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }

    pub(crate) fn from_helper_client(error: ClientError) -> Self {
        match error {
            ClientError::Io(io_error)
                if matches!(
                    io_error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                Self::new("helper_unavailable", "Tunnel helper is not running", true)
            }
            ClientError::Timeout => Self::new(
                "helper_timeout",
                "Tunnel helper did not respond in time",
                true,
            ),
            ClientError::ServerIdentity { .. } => Self::new(
                "helper_identity_mismatch",
                "Tunnel helper identity check failed",
                false,
            ),
            ClientError::ServerProcessIdentity { .. } => Self::new(
                "helper_identity_mismatch",
                "Tunnel helper process identity check failed",
                false,
            ),
            ClientError::ProtocolMismatch { .. } => Self::new(
                "helper_protocol_mismatch",
                "Desktop and tunnel helper versions are incompatible",
                false,
            ),
            ClientError::Helper(helper_error) => Self::new(
                "helper_rejected_request",
                helper_error.message,
                helper_error.retryable,
            ),
            other => Self::new("helper_error", other.to_string(), true),
        }
    }
}

impl HelperSnapshot {
    fn from_status(mode: HelperMode, status: TunnelStatus, stats: TunnelStats) -> Self {
        Self {
            mode,
            reachable: true,
            state: status.state,
            active: status.active,
            stats,
            error: None,
        }
    }

    fn unavailable(mode: HelperMode, error: ClientError) -> Self {
        let error = ControllerError::from_helper_client(error);
        Self {
            mode,
            reachable: false,
            state: HelperState::Idle,
            active: None,
            stats: TunnelStats::default(),
            error: Some(error),
        }
    }
}

pub(crate) struct HelperEndpoints {
    pub(crate) data_dir: PathBuf,
    system_socket: PathBuf,
    user_socket: PathBuf,
    #[cfg(unix)]
    system_uid: u32,
    #[cfg(unix)]
    pub(crate) user_uid: u32,
    #[cfg(unix)]
    pub(crate) user_gid: u32,
    #[cfg(windows)]
    system_pid: Arc<AtomicU32>,
    #[cfg(windows)]
    user_pid: Arc<AtomicU32>,
}

impl HelperEndpoints {
    pub(crate) fn new(data_dir: &Path) -> Self {
        #[cfg(windows)]
        let (system_socket, user_socket) = {
            let nonce = rand::random::<u128>();
            let prefix = format!(r"\\.\pipe\feilian-lite-{}-{nonce:032x}", std::process::id());
            (
                PathBuf::from(format!("{prefix}-system")),
                PathBuf::from(format!("{prefix}-user")),
            )
        };
        #[cfg(not(windows))]
        let (system_socket, user_socket) = (
            data_dir.join("system-helper.sock"),
            data_dir.join("user-helper.sock"),
        );
        Self {
            data_dir: data_dir.to_path_buf(),
            system_socket,
            user_socket,
            #[cfg(unix)]
            system_uid: 0,
            #[cfg(unix)]
            user_uid: current_uid(),
            #[cfg(unix)]
            user_gid: current_gid(),
            #[cfg(windows)]
            system_pid: Arc::new(AtomicU32::new(0)),
            #[cfg(windows)]
            user_pid: Arc::new(AtomicU32::new(0)),
        }
    }

    pub(crate) fn socket(&self, mode: HelperMode) -> &Path {
        match mode {
            HelperMode::SystemSplit => &self.system_socket,
            HelperMode::Socks5 => &self.user_socket,
        }
    }

    pub(crate) fn client(&self, mode: HelperMode) -> HelperClient {
        #[cfg(unix)]
        match mode {
            HelperMode::SystemSplit => HelperClient::new(&self.system_socket, self.system_uid),
            HelperMode::Socks5 => HelperClient::new(&self.user_socket, self.user_uid),
        }
        #[cfg(windows)]
        match mode {
            HelperMode::SystemSplit => {
                HelperClient::new(&self.system_socket, Arc::clone(&self.system_pid))
            }
            HelperMode::Socks5 => HelperClient::new(&self.user_socket, Arc::clone(&self.user_pid)),
        }
    }

    #[cfg(windows)]
    pub(crate) fn set_server_pid(&self, mode: HelperMode, process_id: u32) {
        match mode {
            HelperMode::SystemSplit => &self.system_pid,
            HelperMode::Socks5 => &self.user_pid,
        }
        .store(process_id, Ordering::Release);
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn current_gid() -> u32 {
    unsafe { libc::getegid() }
}

fn helper_binary_path() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("FEILIAN_HELPER_PATH") {
        return PathBuf::from(path);
    }

    let executable_name = if cfg!(windows) {
        "feilian-helper.exe"
    } else {
        "feilian-helper"
    };
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.join(executable_name)))
        .unwrap_or_else(|| PathBuf::from(executable_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret_store::MemorySecretStore;

    #[cfg(unix)]
    #[test]
    fn separates_system_and_user_helper_endpoints() {
        let endpoints = HelperEndpoints::new(Path::new("/tmp/feilian-lite"));

        assert_eq!(
            endpoints.system_socket,
            PathBuf::from("/tmp/feilian-lite/system-helper.sock")
        );
        assert_eq!(
            endpoints.user_socket,
            PathBuf::from("/tmp/feilian-lite/user-helper.sock")
        );
        assert_eq!(endpoints.system_uid, 0);
    }

    #[cfg(windows)]
    #[test]
    fn uses_distinct_random_local_pipe_endpoints() {
        let first = HelperEndpoints::new(Path::new(r"C:\data"));
        let second = HelperEndpoints::new(Path::new(r"C:\data"));

        assert!(first
            .system_socket
            .to_string_lossy()
            .starts_with(r"\\.\pipe\feilian-lite-"));
        assert_ne!(first.system_socket, first.user_socket);
        assert_ne!(first.system_socket, second.system_socket);
    }

    #[test]
    fn system_tunnel_uses_platform_interface_name() {
        let config = WgConf {
            address: "10.0.0.2/32".to_string(),
            address6: String::new(),
            peer_address: "192.0.2.1:51820".to_string(),
            mtu: 1420,
            public_key: "public".to_string(),
            private_key: "private".to_string(),
            peer_key: "peer".to_string(),
            allowed_ips: vec!["10.0.0.0/8".to_string()],
            routes: vec!["10.0.0.0/8".to_string()],
            dns: "10.0.0.53".to_string(),
            protocol: 0,
        };

        let spec = tunnel_spec(HelperMode::SystemSplit, "node-a".to_string(), &config);

        assert_eq!(
            spec.interface_name,
            tunnel_interface_name(HelperMode::SystemSplit)
        );

        let socks_spec = tunnel_spec(HelperMode::Socks5, "node-a".to_string(), &config);
        assert_eq!(socks_spec.interface_name, "feilian-lite");
    }

    #[test]
    fn maps_missing_helper_to_stable_retryable_error() {
        let error = ControllerError::from_helper_client(ClientError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "missing",
        )));

        assert_eq!(error.code, "helper_unavailable");
        assert!(error.retryable);
    }

    #[tokio::test]
    async fn auth_status_is_unconfigured_without_a_profile() {
        let directory = tempfile::tempdir().unwrap();
        let controller = AppController::new(directory.path());

        let status = controller.auth_status().await.unwrap();

        assert!(!status.configured);
        assert!(!status.authenticated);
        assert!(status.nodes.is_empty());
    }

    #[tokio::test]
    async fn migrates_plaintext_profile_secrets_and_reloads_from_secure_storage() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile.json");
        let created = Config::create_desktop_profile(
            &profile,
            "company-code".to_string(),
            "https://vpn.example.test".to_string(),
            PLATFORM_LARK.to_string(),
        )
        .await
        .unwrap();
        let private_key = created.private_key.unwrap();
        let mut legacy_profile: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&profile).unwrap()).unwrap();
        legacy_profile["private_key"] = private_key.clone().into();
        legacy_profile["code"] = "totp-secret".into();
        std::fs::write(
            &profile,
            serde_json::to_string_pretty(&legacy_profile).unwrap(),
        )
        .unwrap();

        let secrets = Arc::new(MemorySecretStore::new());
        let controller = AppController::with_secret_store(directory.path(), secrets.clone());
        let status = controller.auth_status().await.unwrap();

        assert!(status.configured);
        assert_eq!(
            secrets.get(WIREGUARD_PRIVATE_KEY).unwrap().as_deref(),
            Some(private_key.as_str())
        );
        assert_eq!(
            secrets.get(TOTP_SECRET).unwrap().as_deref(),
            Some("totp-secret")
        );
        let migrated = std::fs::read_to_string(&profile).unwrap();
        assert!(!migrated.contains("private_key"));
        assert!(!migrated.contains("totp-secret"));

        let reloaded = AppController::with_secret_store(directory.path(), secrets);
        assert!(reloaded.auth_status().await.unwrap().configured);
    }

    #[tokio::test]
    async fn rejects_redacted_profile_when_secure_private_key_is_missing() {
        let directory = tempfile::tempdir().unwrap();
        Config::create_desktop_profile(
            directory.path().join("profile.json"),
            "company-code".to_string(),
            "https://vpn.example.test".to_string(),
            PLATFORM_LARK.to_string(),
        )
        .await
        .unwrap();
        let controller =
            AppController::with_secret_store(directory.path(), Arc::new(MemorySecretStore::new()));

        let error = controller.auth_status().await.unwrap_err();

        assert_eq!(error.code, "secret_missing");
    }

    #[tokio::test]
    #[ignore = "requires a live tenant, persisted desktop login, and built helper"]
    async fn live_socks5_connects_and_cleans_up() {
        let data_dir = std::env::var("FEILIAN_LIVE_DATA_DIR")
            .expect("FEILIAN_LIVE_DATA_DIR must point to the desktop data directory");
        std::env::var("FEILIAN_HELPER_PATH")
            .expect("FEILIAN_HELPER_PATH must point to a built helper");
        let controller = AppController::new(data_dir);

        controller.initialize().await.unwrap();
        let auth = controller.auth_refresh_nodes().await.unwrap();
        assert!(auth.authenticated);
        let node = auth.nodes.first().expect("live tenant has no VPN nodes");

        let connected = controller
            .connect(HelperMode::Socks5, node.id)
            .await
            .unwrap();
        assert!(connected.reachable);
        assert_eq!(connected.state, HelperState::Running);

        let cleaned = controller.cleanup(HelperMode::Socks5).await;
        assert!(cleaned.reachable);
        assert_eq!(cleaned.state, HelperState::Idle);
    }

    #[tokio::test]
    #[ignore = "requires a live tenant, graphical elevation, and built helper"]
    async fn live_system_tunnel_connects_and_cleans_up() {
        let data_dir = std::env::var("FEILIAN_LIVE_DATA_DIR")
            .expect("FEILIAN_LIVE_DATA_DIR must point to the desktop data directory");
        std::env::var("FEILIAN_HELPER_PATH")
            .expect("FEILIAN_HELPER_PATH must point to a built helper");
        let controller = AppController::new(data_dir);

        controller.initialize().await.unwrap();
        let auth = controller.auth_refresh_nodes().await.unwrap();
        assert!(auth.authenticated);
        let node = auth
            .nodes
            .iter()
            .find(|node| node.available)
            .expect("live tenant has no reachable VPN nodes");

        let connected = controller
            .connect(HelperMode::SystemSplit, node.id)
            .await
            .unwrap();
        assert!(connected.reachable);
        assert_eq!(connected.state, HelperState::Running);

        let cleaned = controller.cleanup(HelperMode::SystemSplit).await;
        assert!(cleaned.reachable);
        assert_eq!(cleaned.state, HelperState::Idle);
    }

    #[test]
    fn qr_challenge_does_not_expose_polling_token() {
        let challenge = QrChallenge {
            login_url: "https://example.test/login".to_string(),
            token: "secret-token".to_string(),
            expires_at_unix: Some(100),
        };

        let desktop = DesktopQrChallenge::from(&challenge);
        let json = serde_json::to_string(&desktop).unwrap();

        assert!(!json.contains("secret-token"));
        assert!(json.contains("https://example.test/login"));
    }

    #[test]
    fn socks5_tunnel_spec_never_contains_system_routes() {
        let config = WgConf {
            address: "10.0.0.2/32".to_string(),
            address6: String::new(),
            peer_address: "192.0.2.1:51820".to_string(),
            mtu: 1420,
            public_key: "public".to_string(),
            private_key: "private".to_string(),
            peer_key: "peer".to_string(),
            allowed_ips: vec!["10.0.0.0/8".to_string()],
            routes: vec!["10.0.0.0/8".to_string()],
            dns: "10.0.0.53".to_string(),
            protocol: 0,
        };

        let spec = tunnel_spec(HelperMode::Socks5, "node-a".to_string(), &config);

        assert!(spec.routes.is_empty());
        assert!(matches!(spec.mode, IpcTunnelMode::Socks5 { .. }));
    }

    #[test]
    fn desktop_node_exposes_names_endpoint_latency_and_availability() {
        let node = AuthNode::from(VpnNode {
            id: 7,
            name: "上海节点".to_string(),
            en_name: "Shanghai-01".to_string(),
            ip: "2001:db8::7".to_string(),
            api_port: 443,
            vpn_port: 51820,
            protocol_mode: 2,
            latency_ms: Some(36),
        });

        assert_eq!(node.name, "上海节点");
        assert_eq!(node.english_name.as_deref(), Some("Shanghai-01"));
        assert_eq!(node.address, "[2001:db8::7]:51820");
        assert_eq!(node.protocol, "udp");
        assert_eq!(node.latency_ms, Some(36));
        assert!(node.available);
    }
}
