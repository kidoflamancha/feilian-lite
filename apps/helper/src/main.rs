#[cfg(unix)]
use std::error::Error;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use feilian_helper::{LibwgBackend, Supervisor, UnixServer};

#[cfg(unix)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse(std::env::args().skip(1))?;
    let server = UnixServer::bind(options.socket, options.owner_uid, options.owner_gid)?;
    let mut supervisor = Supervisor::new(LibwgBackend::default());

    loop {
        tokio::select! {
            result = server.accept_once(&mut supervisor) => {
                if let Err(error) = result {
                    eprintln!("helper request failed: {error}");
                }
            }
            result = wait_for_shutdown_signal() => {
                result?;
                break;
            }
            _ = wait_for_parent_exit(options.parent_pid) => break,
        }
    }
    Ok(())
}

#[cfg(unix)]
struct Options {
    socket: PathBuf,
    owner_uid: u32,
    owner_gid: u32,
    parent_pid: u32,
}

#[cfg(unix)]
impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut socket = None;
        let mut owner_uid = None;
        let mut owner_gid = None;
        let mut parent_pid = None;
        while let Some(argument) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {argument}"))?;
            match argument.as_str() {
                "--socket" => socket = Some(PathBuf::from(value)),
                "--owner-uid" => owner_uid = Some(value.parse()?),
                "--owner-gid" => owner_gid = Some(value.parse()?),
                "--parent-pid" => parent_pid = Some(value.parse()?),
                _ => return Err(format!("unknown argument: {argument}").into()),
            }
        }
        Ok(Self {
            socket: socket.ok_or("--socket is required")?,
            owner_uid: owner_uid.ok_or("--owner-uid is required")?,
            owner_gid: owner_gid.ok_or("--owner-gid is required")?,
            parent_pid: parent_pid.ok_or("--parent-pid is required")?,
        })
    }
}

#[cfg(unix)]
async fn wait_for_parent_exit(parent_pid: u32) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    loop {
        interval.tick().await;
        if !process_exists(parent_pid) {
            return;
        }
    }
}

#[cfg(unix)]
fn process_exists(process_id: u32) -> bool {
    let result = unsafe { libc::kill(process_id as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> std::io::Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn parses_required_parent_process() {
        let options = Options::parse(
            [
                "--socket",
                "/tmp/helper.sock",
                "--owner-uid",
                "1000",
                "--owner-gid",
                "1000",
                "--parent-pid",
                "42",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .unwrap();

        assert_eq!(options.parent_pid, 42);
    }

    #[test]
    fn current_process_is_detected_as_alive() {
        assert!(process_exists(std::process::id()));
    }
}

#[cfg(windows)]
fn main() {
    eprintln!("Windows named-pipe transport is not implemented yet");
    std::process::exit(2);
}
