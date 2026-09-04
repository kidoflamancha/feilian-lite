use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use feilian_helper_client::{ClientError, HelperClient};
use tokio::time::{sleep, Instant};

use crate::controller::{ControllerError, HelperEndpoints, HelperMode};
#[cfg(target_os = "linux")]
use crate::linux_authorization::authorization_flag;

const USER_HELPER_START_TIMEOUT: Duration = Duration::from_secs(5);
const ELEVATED_HELPER_START_TIMEOUT: Duration = Duration::from_secs(60);
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

pub struct HelperLauncher {
    helper_path: PathBuf,
    children: Mutex<HelperChildren>,
}

impl HelperLauncher {
    pub fn new(helper_path: impl Into<PathBuf>) -> Self {
        Self {
            helper_path: helper_path.into(),
            children: Mutex::new(HelperChildren::default()),
        }
    }

    pub async fn ensure_running(
        &self,
        mode: HelperMode,
        endpoints: &HelperEndpoints,
    ) -> Result<HelperClient, ControllerError> {
        #[cfg(windows)]
        if self.child_exit_status(mode)?.is_some() {
            endpoints.set_server_pid(mode, 0);
        }
        let client = endpoints.client(mode);
        if client.hello(env!("CARGO_PKG_VERSION")).await.is_ok() {
            return Ok(client);
        }

        secure_runtime_directory(&endpoints.data_dir)?;
        self.spawn_if_needed(mode, endpoints)?;
        let deadline = Instant::now() + helper_start_timeout(mode);
        loop {
            match client.hello(env!("CARGO_PKG_VERSION")).await {
                Ok(_) => return Ok(client),
                Err(error) if helper_is_unavailable(&error) && Instant::now() < deadline => {
                    if let Some(status) = self.child_exit_status(mode)? {
                        #[cfg(windows)]
                        endpoints.set_server_pid(mode, 0);
                        return Err(helper_exit_error(&status));
                    }
                    sleep(RETRY_INTERVAL).await;
                }
                Err(error) if helper_is_unavailable(&error) => {
                    return Err(ControllerError::new(
                        "helper_start_timeout",
                        "Tunnel helper did not become ready",
                        true,
                    ));
                }
                Err(error) => return Err(ControllerError::from_helper_client(error)),
            }
        }
    }

    fn spawn_if_needed(
        &self,
        mode: HelperMode,
        endpoints: &HelperEndpoints,
    ) -> Result<(), ControllerError> {
        if !self.helper_path.is_file() {
            return Err(ControllerError::new(
                "helper_binary_missing",
                format!(
                    "Tunnel helper was not found at {}",
                    self.helper_path.display()
                ),
                false,
            ));
        }
        #[cfg(windows)]
        if mode == HelperMode::SystemSplit
            && !self.helper_path.with_file_name("wintun.dll").is_file()
        {
            return Err(ControllerError::new(
                "wintun_missing",
                "wintun.dll was not found next to the tunnel helper",
                false,
            ));
        }
        #[cfg(target_os = "macos")]
        if mode == HelperMode::SystemSplit {
            verify_macos_elevation_target(&self.helper_path)?;
        }

        let mut children = self.children.lock().map_err(|_| {
            ControllerError::new(
                "helper_launcher_failed",
                "Helper launcher lock is poisoned",
                false,
            )
        })?;
        let slot = children.slot(mode);
        if let Some(child) = slot.as_mut() {
            match child.try_wait() {
                Ok(None) => return Ok(()),
                Ok(Some(_)) => *slot = None,
                Err(error) => {
                    return Err(ControllerError::new(
                        "helper_launcher_failed",
                        error.to_string(),
                        true,
                    ));
                }
            }
        }

        let plan = launch_plan(mode, &self.helper_path, endpoints)?;
        let child = spawn_plan(plan)?;
        #[cfg(windows)]
        endpoints.set_server_pid(mode, child.id());
        *slot = Some(child);
        Ok(())
    }

    fn child_exit_status(&self, mode: HelperMode) -> Result<Option<String>, ControllerError> {
        let mut children = self.children.lock().map_err(|_| {
            ControllerError::new(
                "helper_launcher_failed",
                "Helper launcher lock is poisoned",
                false,
            )
        })?;
        let slot = children.slot(mode);
        let Some(child) = slot.as_mut() else {
            return Ok(None);
        };
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() && child.may_detach_on_success() {
                    *slot = None;
                    return Ok(None);
                }
                *slot = None;
                Ok(Some(status.to_string()))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(ControllerError::new(
                "helper_launcher_failed",
                error.to_string(),
                true,
            )),
        }
    }
}

fn helper_start_timeout(mode: HelperMode) -> Duration {
    match mode {
        HelperMode::SystemSplit => ELEVATED_HELPER_START_TIMEOUT,
        HelperMode::Socks5 => USER_HELPER_START_TIMEOUT,
    }
}

fn helper_exit_error(status: &str) -> ControllerError {
    #[cfg(target_os = "linux")]
    if status == "exit status: 126" {
        return ControllerError::new(
            "authorization_cancelled",
            "管理员授权已取消，系统分流未启动",
            true,
        );
    }
    #[cfg(target_os = "linux")]
    if status == "exit status: 127" {
        return ControllerError::new(
            "authorization_unavailable",
            "无法完成管理员授权，请确认 Polkit 认证代理正在运行",
            true,
        );
    }
    ControllerError::new(
        "helper_launch_failed",
        format!("Tunnel helper exited before startup ({status})"),
        true,
    )
}

#[derive(Default)]
struct HelperChildren {
    system: Option<TrackedChild>,
    user: Option<TrackedChild>,
}

impl HelperChildren {
    fn slot(&mut self, mode: HelperMode) -> &mut Option<TrackedChild> {
        match mode {
            HelperMode::SystemSplit => &mut self.system,
            HelperMode::Socks5 => &mut self.user,
        }
    }
}

struct LaunchPlan {
    program: PathBuf,
    args: Vec<String>,
    may_detach_on_success: bool,
    #[cfg(windows)]
    elevated: bool,
}

fn launch_plan(
    mode: HelperMode,
    helper_path: &Path,
    endpoints: &HelperEndpoints,
) -> Result<LaunchPlan, ControllerError> {
    let helper_args = helper_arguments(mode, endpoints);
    match mode {
        HelperMode::Socks5 => Ok(LaunchPlan {
            program: helper_path.to_path_buf(),
            args: helper_args,
            may_detach_on_success: false,
            #[cfg(windows)]
            elevated: false,
        }),
        HelperMode::SystemSplit => elevated_launch_plan(helper_path, helper_args),
    }
}

#[cfg(unix)]
fn helper_arguments(mode: HelperMode, endpoints: &HelperEndpoints) -> Vec<String> {
    vec![
        "--socket".to_string(),
        endpoints.socket(mode).to_string_lossy().into_owned(),
        "--owner-uid".to_string(),
        endpoints.user_uid.to_string(),
        "--owner-gid".to_string(),
        endpoints.user_gid.to_string(),
        "--parent-pid".to_string(),
        std::process::id().to_string(),
    ]
}

#[cfg(windows)]
fn helper_arguments(mode: HelperMode, endpoints: &HelperEndpoints) -> Vec<String> {
    vec![
        "--pipe".to_string(),
        endpoints.socket(mode).to_string_lossy().into_owned(),
        "--parent-pid".to_string(),
        std::process::id().to_string(),
    ]
}

#[cfg(target_os = "linux")]
fn elevated_launch_plan(
    helper_path: &Path,
    helper_args: Vec<String>,
) -> Result<LaunchPlan, ControllerError> {
    if graphical_session_is_remote() {
        return terminal_elevated_launch_plan(helper_path, helper_args);
    }
    Ok(direct_pkexec_launch_plan(helper_path, helper_args))
}

#[cfg(target_os = "linux")]
fn direct_pkexec_launch_plan(helper_path: &Path, mut helper_args: Vec<String>) -> LaunchPlan {
    helper_args.insert(0, helper_path.to_string_lossy().into_owned());
    helper_args.insert(0, "--disable-internal-agent".to_string());
    LaunchPlan {
        program: PathBuf::from("pkexec"),
        args: helper_args,
        may_detach_on_success: false,
    }
}

#[cfg(target_os = "linux")]
fn graphical_session_is_remote() -> bool {
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return true;
    }
    let user_id = unsafe { libc::geteuid() }.to_string();
    let display_session = Command::new("loginctl")
        .args(["show-user", &user_id, "-p", "Display", "--value"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|session| session.trim().to_string())
        .filter(|session| !session.is_empty());
    let Some(session) = display_session else {
        return false;
    };
    Command::new("loginctl")
        .args(["show-session", &session, "-p", "Remote", "--value"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|remote| remote.trim() == "yes")
}

#[cfg(target_os = "linux")]
fn terminal_elevated_launch_plan(
    helper_path: &Path,
    helper_args: Vec<String>,
) -> Result<LaunchPlan, ControllerError> {
    #[cfg(test)]
    let desktop_path = std::env::var_os("FEILIAN_AUTHORIZATION_RUNNER_PATH")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(|error| ControllerError::new("desktop_path_invalid", error.to_string(), false))?;
    #[cfg(not(test))]
    let desktop_path = std::env::current_exe()
        .map_err(|error| ControllerError::new("desktop_path_invalid", error.to_string(), false))?;
    let candidates = [
        ("/usr/bin/kgx", TerminalKind::Kgx),
        ("/usr/bin/gnome-terminal", TerminalKind::GnomeTerminal),
        ("/usr/bin/konsole", TerminalKind::Konsole),
        ("/usr/bin/xterm", TerminalKind::Xterm),
    ];
    let (terminal, kind) = candidates
        .into_iter()
        .find(|(path, _)| Path::new(path).is_file())
        .ok_or_else(|| {
            ControllerError::new(
                "authorization_terminal_missing",
                "远程图形会话需要终端完成管理员授权，但未找到受支持的终端应用",
                false,
            )
        })?;
    Ok(terminal_launch_plan(
        terminal,
        kind,
        &desktop_path,
        helper_path,
        helper_args,
    ))
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum TerminalKind {
    Kgx,
    GnomeTerminal,
    Konsole,
    Xterm,
}

#[cfg(target_os = "linux")]
fn terminal_launch_plan(
    terminal: &str,
    kind: TerminalKind,
    desktop_path: &Path,
    helper_path: &Path,
    helper_args: Vec<String>,
) -> LaunchPlan {
    let mut args = match kind {
        TerminalKind::Kgx => vec![
            "--wait".to_string(),
            "--title=Feilian Lite 管理员授权".to_string(),
            "-e".to_string(),
            desktop_path.to_string_lossy().into_owned(),
        ],
        TerminalKind::GnomeTerminal => vec![
            "--wait".to_string(),
            "--title=Feilian Lite 管理员授权".to_string(),
            "--".to_string(),
            desktop_path.to_string_lossy().into_owned(),
        ],
        TerminalKind::Konsole => vec![
            "--separate".to_string(),
            "--nofork".to_string(),
            "-p".to_string(),
            "tabtitle=Feilian Lite 管理员授权".to_string(),
            "-e".to_string(),
            desktop_path.to_string_lossy().into_owned(),
        ],
        TerminalKind::Xterm => vec![
            "-T".to_string(),
            "Feilian Lite 管理员授权".to_string(),
            "-e".to_string(),
            desktop_path.to_string_lossy().into_owned(),
        ],
    };
    args.push(authorization_flag().to_string());
    args.push(helper_path.to_string_lossy().into_owned());
    args.extend(helper_args);
    LaunchPlan {
        program: PathBuf::from(terminal),
        args,
        may_detach_on_success: true,
    }
}

#[cfg(target_os = "macos")]
fn elevated_launch_plan(
    helper_path: &Path,
    helper_args: Vec<String>,
) -> Result<LaunchPlan, ControllerError> {
    const SCRIPT: &str = r#"on run argv
set commandText to "exec"
repeat with argumentText in argv
set commandText to commandText & space & quoted form of (contents of argumentText)
end repeat
set commandText to commandText & " </dev/null >/dev/null 2>&1"
do shell script commandText with administrator privileges
end run"#;

    let helper = helper_path.to_str().ok_or_else(|| {
        ControllerError::new(
            "helper_path_invalid",
            "Helper path is not valid Unicode",
            false,
        )
    })?;
    let mut args = vec!["-e".to_string(), SCRIPT.to_string(), helper.to_string()];
    args.extend(helper_args);
    Ok(LaunchPlan {
        program: PathBuf::from("/usr/bin/osascript"),
        args,
        may_detach_on_success: false,
    })
}

#[cfg(windows)]
fn elevated_launch_plan(
    helper_path: &Path,
    helper_args: Vec<String>,
) -> Result<LaunchPlan, ControllerError> {
    Ok(LaunchPlan {
        program: helper_path.to_path_buf(),
        args: helper_args,
        may_detach_on_success: false,
        elevated: true,
    })
}

enum TrackedChild {
    Direct {
        child: Child,
        may_detach_on_success: bool,
    },
    #[cfg(windows)]
    Elevated(WindowsElevatedChild),
}

impl TrackedChild {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self {
            Self::Direct { child, .. } => child.try_wait(),
            #[cfg(windows)]
            Self::Elevated(child) => child.try_wait(),
        }
    }

    fn may_detach_on_success(&self) -> bool {
        match self {
            Self::Direct {
                may_detach_on_success,
                ..
            } => *may_detach_on_success,
            #[cfg(windows)]
            Self::Elevated(_) => false,
        }
    }

    #[cfg(windows)]
    fn id(&self) -> u32 {
        match self {
            Self::Direct { child, .. } => child.id(),
            Self::Elevated(child) => child.process_id,
        }
    }
}

fn spawn_plan(plan: LaunchPlan) -> Result<TrackedChild, ControllerError> {
    #[cfg(windows)]
    if plan.elevated {
        return WindowsElevatedChild::spawn(&plan.program, &plan.args)
            .map(TrackedChild::Elevated)
            .map_err(|error| {
                ControllerError::new("helper_launch_failed", error.to_string(), true)
            });
    }

    let mut command = Command::new(&plan.program);
    command
        .args(&plan.args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .spawn()
        .map(|child| TrackedChild::Direct {
            child,
            may_detach_on_success: plan.may_detach_on_success,
        })
        .map_err(|error| ControllerError::new("helper_launch_failed", error.to_string(), true))
}

#[cfg(target_os = "macos")]
fn verify_macos_elevation_target(helper_path: &Path) -> Result<(), ControllerError> {
    let desktop_path = std::env::current_exe()
        .map_err(|error| ControllerError::new("desktop_path_invalid", error.to_string(), false))?;
    let desktop_path = desktop_path
        .canonicalize()
        .map_err(|error| ControllerError::new("desktop_path_invalid", error.to_string(), false))?;
    let helper_path = helper_path
        .canonicalize()
        .map_err(|error| ControllerError::new("helper_path_invalid", error.to_string(), false))?;
    let applications = Path::new("/Applications");
    if !desktop_path.starts_with(applications) || !helper_path.starts_with(applications) {
        return Err(ControllerError::new(
            "trusted_install_required",
            "Install Feilian Lite in /Applications before using system tunnel mode",
            false,
        ));
    }

    let desktop_team = verified_signing_team(&desktop_path)?;
    let helper_team = verified_signing_team(&helper_path)?;
    if desktop_team != helper_team {
        return Err(ControllerError::new(
            "helper_identity_mismatch",
            "Desktop and tunnel helper signatures do not match",
            false,
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn verified_signing_team(path: &Path) -> Result<String, ControllerError> {
    let verified = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--verbose=0"])
        .arg(path)
        .status()
        .map_err(|error| {
            ControllerError::new("signature_check_failed", error.to_string(), false)
        })?;
    if !verified.success() {
        return Err(ControllerError::new(
            "signature_check_failed",
            "The application or tunnel helper has an invalid code signature",
            false,
        ));
    }

    let details = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=4"])
        .arg(path)
        .output()
        .map_err(|error| {
            ControllerError::new("signature_check_failed", error.to_string(), false)
        })?;
    signing_team(&details.stderr).ok_or_else(|| {
        ControllerError::new(
            "signature_check_failed",
            "A Developer ID TeamIdentifier is required for system tunnel mode",
            false,
        )
    })
}

#[cfg(any(target_os = "macos", test))]
fn signing_team(output: &[u8]) -> Option<String> {
    String::from_utf8_lossy(output)
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .map(str::trim)
        .filter(|team| !team.is_empty() && *team != "not set")
        .map(str::to_string)
}

#[cfg(windows)]
struct WindowsElevatedChild {
    handle: isize,
    process_id: u32,
}

#[cfg(windows)]
impl WindowsElevatedChild {
    fn spawn(program: &Path, args: &[String]) -> std::io::Result<Self> {
        use std::mem;
        use std::os::windows::ffi::OsStrExt;
        use std::ptr;
        use windows_sys::Win32::System::Threading::GetProcessId;
        use windows_sys::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

        let verb = "runas\0".encode_utf16().collect::<Vec<_>>();
        let program = program
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let parameters = args
            .iter()
            .map(|argument| quote_windows_argument(argument))
            .collect::<Vec<_>>()
            .join(" ")
            .encode_utf16()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let mut info: SHELLEXECUTEINFOW = unsafe { mem::zeroed() };
        info.cbSize = mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOASYNC | SEE_MASK_NOCLOSEPROCESS;
        info.hwnd = ptr::null_mut();
        info.lpVerb = verb.as_ptr();
        info.lpFile = program.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_HIDE;

        if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let process_id = unsafe { GetProcessId(info.hProcess) };
        if process_id == 0 {
            unsafe { windows_sys::Win32::Foundation::CloseHandle(info.hProcess) };
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self {
            handle: info.hProcess as isize,
            process_id,
        })
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        use std::os::windows::process::ExitStatusExt;
        use windows_sys::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};

        let handle = self.handle as _;
        match unsafe { WaitForSingleObject(handle, 0) } {
            WAIT_TIMEOUT => Ok(None),
            WAIT_OBJECT_0 => {
                let mut exit_code = 0;
                if unsafe { GetExitCodeProcess(handle, &mut exit_code) } == 0 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(Some(std::process::ExitStatus::from_raw(exit_code)))
                }
            }
            _ => Err(std::io::Error::last_os_error()),
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsElevatedChild {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle as _) };
    }
}

#[cfg(windows)]
fn quote_windows_argument(argument: &str) -> String {
    if !argument.is_empty()
        && !argument
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return argument.to_string();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in argument.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(character);
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

fn helper_is_unavailable(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::ServerIdentity { .. }
            | ClientError::ServerProcessIdentity { .. }
            | ClientError::RequestMismatch { .. }
            | ClientError::UnexpectedResponse(_)
            | ClientError::Timeout
    ) || matches!(
        error,
        ClientError::Io(io_error)
            if matches!(
                io_error.kind(),
                std::io::ErrorKind::NotFound
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
            )
    )
}

fn secure_runtime_directory(path: &Path) -> Result<(), ControllerError> {
    fs::create_dir_all(path)
        .map_err(|error| ControllerError::new("helper_runtime_failed", error.to_string(), true))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|error| {
            ControllerError::new("helper_runtime_failed", error.to_string(), true)
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoints(path: &Path) -> HelperEndpoints {
        HelperEndpoints::new(path)
    }

    #[cfg(unix)]
    #[test]
    fn socks5_launches_helper_without_elevation() {
        let endpoints = endpoints(Path::new("/tmp/feilian"));
        let helper = Path::new("/opt/feilian/feilian-helper");

        let plan = launch_plan(HelperMode::Socks5, helper, &endpoints).unwrap();

        assert_eq!(plan.program, helper);
        assert_eq!(plan.args[0], "--socket");
        assert!(plan.args[1].ends_with("user-helper.sock"));
        assert!(plan.args.contains(&"--parent-pid".to_string()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn system_tunnel_uses_pkexec_with_fixed_helper_arguments() {
        let endpoints = endpoints(Path::new("/tmp/feilian"));
        let helper = Path::new("/opt/feilian/feilian-helper");

        let plan = direct_pkexec_launch_plan(
            helper,
            helper_arguments(HelperMode::SystemSplit, &endpoints),
        );

        assert_eq!(plan.program, Path::new("pkexec"));
        assert_eq!(plan.args[0], "--disable-internal-agent");
        assert_eq!(plan.args[1], "/opt/feilian/feilian-helper");
        assert_eq!(plan.args[2], "--socket");
        assert!(plan.args[3].ends_with("system-helper.sock"));
        assert!(plan.args.contains(&"--parent-pid".to_string()));
        assert!(!plan.may_detach_on_success);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn system_tunnel_uses_parameterized_apple_script_elevation() {
        let endpoints = endpoints(Path::new("/tmp/feilian"));
        let helper = Path::new("/Applications/Feilian Lite.app/Contents/MacOS/feilian-helper");

        let plan = launch_plan(HelperMode::SystemSplit, helper, &endpoints).unwrap();

        assert_eq!(plan.program, Path::new("/usr/bin/osascript"));
        assert_eq!(plan.args[0], "-e");
        assert!(plan.args[1].contains("quoted form of"));
        assert_eq!(plan.args[2], helper.to_string_lossy());
        assert!(plan.args.contains(&"--socket".to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn system_tunnel_uses_uac_with_named_pipe_arguments() {
        let endpoints = endpoints(Path::new(r"C:\data"));
        let helper = Path::new(r"C:\Program Files\Feilian Lite\feilian-helper.exe");

        let plan = launch_plan(HelperMode::SystemSplit, helper, &endpoints).unwrap();

        assert_eq!(plan.program, helper);
        assert!(plan.elevated);
        assert_eq!(plan.args[0], "--pipe");
        assert!(plan.args[1].starts_with(r"\\.\pipe\feilian-lite-"));
        assert!(plan.args.contains(&"--parent-pid".to_string()));
    }

    #[test]
    fn runtime_directory_is_owner_only() {
        let parent = tempfile::tempdir().unwrap();
        let directory = parent.path().join("runtime");

        secure_runtime_directory(&directory).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn elevated_helper_allows_time_for_interactive_authorization() {
        assert_eq!(
            helper_start_timeout(HelperMode::Socks5),
            Duration::from_secs(5)
        );
        assert_eq!(
            helper_start_timeout(HelperMode::SystemSplit),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn parses_nonempty_macos_signing_team() {
        assert_eq!(
            signing_team(b"Identifier=dev.feilian.lite\nTeamIdentifier=ABCDE12345\n"),
            Some("ABCDE12345".to_string())
        );
        assert_eq!(signing_team(b"TeamIdentifier=not set\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn maps_pkexec_authorization_exit_codes() {
        assert_eq!(
            helper_exit_error("exit status: 126").code,
            "authorization_cancelled"
        );
        assert_eq!(
            helper_exit_error("exit status: 127").code,
            "authorization_unavailable"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn remote_session_uses_visible_terminal_authorization() {
        let plan = terminal_launch_plan(
            "/usr/bin/kgx",
            TerminalKind::Kgx,
            Path::new("/opt/feilian/feilian-desktop"),
            Path::new("/opt/feilian/feilian-helper"),
            vec!["--socket".to_string(), "/tmp/helper.sock".to_string()],
        );

        assert_eq!(plan.program, Path::new("/usr/bin/kgx"));
        assert_eq!(plan.args[0], "--wait");
        assert_eq!(plan.args[2], "-e");
        assert_eq!(plan.args[3], "/opt/feilian/feilian-desktop");
        assert_eq!(plan.args[4], "--authorize-helper");
        assert_eq!(plan.args[5], "/opt/feilian/feilian-helper");
        assert_eq!(plan.args[6], "--socket");
        assert!(plan.may_detach_on_success);
    }

    #[cfg(windows)]
    #[test]
    fn quotes_windows_uac_arguments() {
        assert_eq!(quote_windows_argument("plain"), "plain");
        assert_eq!(
            quote_windows_argument(r"C:\Program Files\Feilian Lite\"),
            r#""C:\Program Files\Feilian Lite\\""#
        );
        assert_eq!(quote_windows_argument("a\"b"), r#""a\"b""#);
    }
}
