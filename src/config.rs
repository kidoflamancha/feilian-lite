use std::fmt;
use std::path::Path;
use tokio::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::state::State;
use crate::utils;

const DEFAULT_DEVICE_NAME: &str = "DollarOS";
const DEFAULT_INTERFACE_NAME: &str = "corplink";

pub const PLATFORM_LDAP: &str = "ldap";
pub const PLATFORM_CORPLINK: &str = "feilian";
// new feilian login that uses the v1 API (/api/v1/login with an AES-encrypted
// password), as served by the newer feilian backend. opt-in via config.
pub const PLATFORM_CORPLINK_V1: &str = "feilian_v1";
pub const PLATFORM_OIDC: &str = "OIDC";
// aka feishu
pub const PLATFORM_LARK: &str = "lark";
#[allow(dead_code)]
pub const PLATFORM_WEIXIN: &str = "weixin";
// aka dingding
#[allow(dead_code)]
pub const PLATFORM_DING_TALK: &str = "dingtalk";
// unknown
#[allow(dead_code)]
pub const PLATFORM_AAD: &str = "aad";

pub const STRATEGY_LATENCY: &str = "latency";
pub const STRATEGY_DEFAULT: &str = "default";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    /// Only intranet routes returned by the server (mimics official split mode).
    #[default]
    Split,
    /// Full-tunnel routes from the server (typically 0.0.0.0/0, ::/0).
    Full,
}

impl fmt::Display for RouteMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RouteMode::Split => write!(f, "split"),
            RouteMode::Full => write!(f, "full"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Config {
    pub company_name: String,
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub device_name: Option<String>,
    pub device_id: Option<String>,
    pub public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    pub server: Option<String>,
    pub interface_name: Option<String>,
    pub debug_wg: Option<bool>,
    #[serde(skip_serializing)]
    pub conf_file: Option<String>,
    pub state: Option<State>,
    pub vpn_server_name: Option<String>,
    pub vpn_select_strategy: Option<String>,
    pub use_vpn_dns: Option<bool>,
    pub dns_backup_filename: Option<String>,
    pub auto_setup_routes: Option<bool>,
    /// "split" (default) or "full". Selects which route list from the server to apply.
    pub route_mode: Option<RouteMode>,
    /// Optional CIDRs added to the server-provided routes before route filters.
    /// Unlike `vpn_allowed_routes`, this expands the route set. The combined routes
    /// are then restricted by `vpn_allowed_routes` and `vpn_disallowed_routes`.
    pub vpn_additional_routes: Option<Vec<String>>,
    /// Optional hostnames resolved on every connection. Resolved addresses are appended
    /// as host routes before route filters.
    pub vpn_additional_domains: Option<Vec<String>>,
    /// Optional CIDR whitelist intersected with the server and additional routes.
    /// Missing/null preserves the combined routes; an empty list allows no routes.
    pub vpn_allowed_routes: Option<Vec<String>>,
    /// Optional list of CIDR routes to exclude from AllowedIPs / system routes.
    /// Useful in full mode to punch holes for local LAN or the VPN peer IP itself,
    /// avoiding routing loops (e.g. 192.168.1.0/24, 10.0.0.5/32).
    pub vpn_disallowed_routes: Option<Vec<String>>,
    /// When set, run entirely in userspace (gVisor netstack) and expose a SOCKS5
    /// proxy at this listen address (e.g. "0.0.0.0:1080" or "127.0.0.1:1080")
    /// instead of creating a kernel TUN device. No system interface, routes, DNS
    /// changes or root privileges are required. Only TCP CONNECT is supported.
    pub socks5_listen: Option<String>,
    /// Optional SOCKS5 username/password authentication (RFC 1929). When
    /// `socks5_username` is set and non-empty, clients must authenticate with
    /// these credentials; otherwise the proxy accepts connections without auth.
    pub socks5_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socks5_password: Option<String>,
    /// Force the WireGuard transport protocol instead of using the server-advertised
    /// `protocol_mode`. Accepts "udp" or "tcp" (case-insensitive). Some `protocol_mode: 1`
    /// (TCP) gateways also accept WireGuard over UDP -- for those the server even ships a
    /// `protocol_detect_config` (udp<->tcp switch thresholds) in the `/api/vpn/list` entry.
    /// Since WireGuard-over-TCP can collapse to a few KB/s on a lossy uplink (TCP-over-TCP
    /// head-of-line blocking), forcing "udp" can be far faster there. Leave unset to keep the
    /// default (follow server `protocol_mode`: 1 => tcp, otherwise udp).
    pub force_protocol: Option<String>,
    #[serde(skip, default = "persist_secrets_by_default")]
    persist_secrets: bool,
}

fn persist_secrets_by_default() -> bool {
    true
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match serde_json::to_string_pretty(self) {
            Ok(s) => write!(f, "{}", s),
            Err(e) => write!(f, "<invalid config: {e}>"),
        }
    }
}

impl Config {
    pub async fn create_desktop_profile(
        file: impl AsRef<Path>,
        company_name: String,
        server: String,
        platform: String,
    ) -> Result<Config> {
        let file = file.as_ref();
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create profile directory {}", parent.display()))?;
            set_private_directory_permissions(parent)?;
        }

        let (public_key, private_key) = utils::gen_wg_keypair();
        let device_id = hex::encode(rand::random::<[u8; 16]>());
        let conf = Config {
            company_name,
            username: String::new(),
            password: None,
            platform: Some(platform),
            code: None,
            device_name: Some("Feilian Lite".to_string()),
            device_id: Some(device_id),
            public_key: Some(public_key),
            private_key: Some(private_key),
            server: Some(server),
            interface_name: Some("feilian-lite".to_string()),
            debug_wg: Some(false),
            conf_file: Some(file.to_string_lossy().into_owned()),
            state: Some(State::Init),
            vpn_server_name: None,
            vpn_select_strategy: Some(STRATEGY_LATENCY.to_string()),
            use_vpn_dns: Some(false),
            dns_backup_filename: None,
            auto_setup_routes: Some(true),
            route_mode: Some(RouteMode::Split),
            vpn_additional_routes: None,
            vpn_additional_domains: None,
            vpn_allowed_routes: None,
            vpn_disallowed_routes: None,
            socks5_listen: None,
            socks5_username: None,
            socks5_password: None,
            force_protocol: None,
            persist_secrets: false,
        };
        conf.save().await?;
        Ok(conf)
    }

    pub async fn from_file(file: &str) -> Result<Config> {
        Self::load(file, true).await
    }

    pub async fn from_desktop_profile(file: &str) -> Result<Config> {
        Self::load(file, false).await
    }

    async fn load(file: &str, persist_secrets: bool) -> Result<Config> {
        let conf_str = fs::read_to_string(file)
            .await
            .with_context(|| format!("failed to read config file {file}"))?;

        let mut conf: Config = serde_json::from_str(&conf_str[..])
            .with_context(|| format!("failed to parse config file {file}"))?;

        conf.conf_file = Some(file.to_string());
    conf.persist_secrets = persist_secrets;
        let mut update_conf = false;
        if conf.interface_name.is_none() {
            conf.interface_name = Some(DEFAULT_INTERFACE_NAME.to_string());
            update_conf = true;
        }
        if conf.device_name.is_none() {
            conf.device_name = Some(DEFAULT_DEVICE_NAME.to_string());
            update_conf = true;
        }
        if conf.device_id.is_none() {
            let device_name = conf
                .device_name
                .as_ref()
                .context("device name missing when generating device id")?;
            conf.device_id = Some(format!("{:x}", md5::compute(device_name)));
            update_conf = true;
        }
        match &conf.private_key {
            Some(private_key) => match conf.public_key {
                Some(_) => {
                    // both keys exist, do nothing
                }
                None => {
                    // only private key exists, generate public from private
                    let public_key = utils::gen_public_key_from_private(private_key)?;
                    conf.public_key = Some(public_key);
                    update_conf = true;
                }
            },
            None if persist_secrets => {
                // no key exists, generate new
                let (public_key, private_key) = utils::gen_wg_keypair();
                (conf.public_key, conf.private_key) = (Some(public_key), Some(private_key));
                update_conf = true;
            }
            None => {}
        }
        if update_conf && persist_secrets {
            conf.save().await?;
        }
        Ok(conf)
    }

    pub async fn save(&self) -> Result<()> {
        let file = self
            .conf_file
            .as_ref()
            .context("config file path missing")?;
        ensure_private_file(Path::new(file))?;
        let data = if self.persist_secrets {
            serde_json::to_string_pretty(self).context("serialize config")?
        } else {
            let mut persisted = self.clone();
            persisted.password = None;
            persisted.private_key = None;
            persisted.code = None;
            persisted.socks5_password = None;
            serde_json::to_string_pretty(&persisted).context("serialize redacted config")?
        };
        fs::write(file, data)
            .await
            .with_context(|| format!("failed to write config file {file}"))?;
        Ok(())
    }

    pub fn keep_secrets_out_of_profile(&mut self) {
        self.persist_secrets = false;
    }
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure directory {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn ensure_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create private file {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure file {}", path.display()))
}

#[cfg(not(unix))]
fn ensure_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[derive(Serialize, Clone)]
pub struct WgConf {
    // standard wg conf
    pub address: String,
    pub address6: String,
    pub peer_address: String,
    pub mtu: u32,
    pub public_key: String,
    pub private_key: String,
    pub peer_key: String,
    pub allowed_ips: Vec<String>,
    pub routes: Vec<String>,

    // extra confs
    pub dns: String,

    // corplink confs
    pub protocol: i32,
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[tokio::test]
    async fn desktop_profile_is_private_and_excludes_secrets() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("feilian-profile-{unique}"));
        let profile = directory.join("profile.json");

        let created = Config::create_desktop_profile(
            &profile,
            "company-code".to_string(),
            "https://vpn.example.test".to_string(),
            PLATFORM_LARK.to_string(),
        )
        .await
        .unwrap();
        let profile_json = std::fs::read_to_string(&profile).unwrap();

        assert_eq!(created.company_name, "company-code");
        assert_eq!(created.platform.as_deref(), Some(PLATFORM_LARK));
        assert_eq!(created.server.as_deref(), Some("https://vpn.example.test"));
        assert!(created.private_key.is_some());
        assert!(created.public_key.is_some());
        assert!(!profile_json.contains("private_key"));
        assert!(!profile_json.contains("password"));
        assert!(!profile_json.contains("socks5_password"));

        let mut authenticated = created;
        authenticated.code = Some("totp-secret".to_string());
        authenticated.save().await.unwrap();
        let authenticated_json = std::fs::read_to_string(&profile).unwrap();
        assert!(!authenticated_json.contains("totp-secret"));
        assert!(!authenticated_json.contains("\"code\""));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                std::fs::metadata(&directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(&profile)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        std::fs::remove_dir_all(directory).unwrap();
    }
}
