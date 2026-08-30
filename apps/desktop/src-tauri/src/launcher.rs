use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use feilian_helper_client::{ClientError, HelperClient};
use tokio::time::{sleep, Instant};

use crate::controller::{ControllerError, HelperEndpoints, HelperMode};

const START_TIMEOUT: Duration = Duration::from_secs(5);
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
        let client = endpoints.client(mode);
        match client.hello(env!("CARGO_PKG_VERSION")).await {
            Ok(_) => return Ok(client),
            Err(error) if helper_is_unavailable(&error) => {}
            Err(error) => return Err(ControllerError::from_helper_client(error)),
        }

        secure_runtime_directory(&endpoints.data_dir)?;
        self.spawn_if_needed(mode, endpoints)?;
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            match client.hello(env!("CARGO_PKG_VERSION")).await {
                Ok(_) => return Ok(client),
                Err(error) if helper_is_unavailable(&error) && Instant::now() < deadline => {
                    if let Some(status) = self.child_exit_status(mode)? {
                        return Err(ControllerError::new(
                            "helper_launch_failed",
                            format!("Tunnel helper exited before startup ({status})"),
                            true,
                        ));
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
        let mut command = Command::new(&plan.program);
        command
            .args(&plan.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().map_err(|error| {
            ControllerError::new("helper_launch_failed", error.to_string(), true)
        })?;
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

#[derive(Default)]
struct HelperChildren {
    system: Option<Child>,
    user: Option<Child>,
}

impl HelperChildren {
    fn slot(&mut self, mode: HelperMode) -> &mut Option<Child> {
        match mode {
            HelperMode::SystemSplit => &mut self.system,
            HelperMode::Socks5 => &mut self.user,
        }
    }
}

struct LaunchPlan {
    program: PathBuf,
    args: Vec<String>,
}

fn launch_plan(
    mode: HelperMode,
    helper_path: &Path,
    endpoints: &HelperEndpoints,
) -> Result<LaunchPlan, ControllerError> {
    let socket = endpoints.socket(mode).to_string_lossy().into_owned();
    let helper_args = vec![
        "--socket".to_string(),
        socket,
        "--owner-uid".to_string(),
        endpoints.user_uid.to_string(),
        "--owner-gid".to_string(),
        endpoints.user_gid.to_string(),
        "--parent-pid".to_string(),
        std::process::id().to_string(),
    ];
    match mode {
        HelperMode::Socks5 => Ok(LaunchPlan {
            program: helper_path.to_path_buf(),
            args: helper_args,
        }),
        HelperMode::SystemSplit => elevated_launch_plan(helper_path, helper_args),
    }
}

#[cfg(target_os = "linux")]
fn elevated_launch_plan(
    helper_path: &Path,
    mut helper_args: Vec<String>,
) -> Result<LaunchPlan, ControllerError> {
    helper_args.insert(0, helper_path.to_string_lossy().into_owned());
    Ok(LaunchPlan {
        program: PathBuf::from("pkexec"),
        args: helper_args,
    })
}

#[cfg(not(target_os = "linux"))]
fn elevated_launch_plan(
    _helper_path: &Path,
    _helper_args: Vec<String>,
) -> Result<LaunchPlan, ControllerError> {
    Err(ControllerError::new(
        "helper_elevation_unsupported",
        "System tunnel elevation is not implemented on this platform",
        false,
    ))
}

fn helper_is_unavailable(error: &ClientError) -> bool {
    matches!(
        error,
        ClientError::Io(io_error)
            if matches!(
                io_error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
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

        let plan = launch_plan(HelperMode::SystemSplit, helper, &endpoints).unwrap();

        assert_eq!(plan.program, Path::new("pkexec"));
        assert_eq!(plan.args[0], "/opt/feilian/feilian-helper");
        assert_eq!(plan.args[1], "--socket");
        assert!(plan.args[2].ends_with("system-helper.sock"));
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
}
