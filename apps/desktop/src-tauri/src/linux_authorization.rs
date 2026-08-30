use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::Duration;

const AUTHORIZATION_FLAG: &str = "--authorize-helper";
const AGENT_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(5);

pub fn run_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(OsStr::new(AUTHORIZATION_FLAG)) {
        return None;
    }

    let helper = match args.next() {
        Some(helper) => helper,
        None => {
            eprintln!("Feilian Lite authorization runner: helper path is missing");
            return Some(2);
        }
    };
    let helper_args = args.collect::<Vec<_>>();
    let exit_code = match authorize_helper(&helper, &helper_args) {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("Feilian Lite authorization failed: {error}");
            1
        }
    };
    Some(exit_code)
}

fn authorize_helper(helper: &OsStr, helper_args: &[OsString]) -> io::Result<ExitStatus> {
    println!("Feilian Lite 系统分流需要管理员权限。");
    println!("请在下方输入系统管理员密码；输入内容不会显示，也不会被 Feilian Lite 保存。\n");
    println!("授权成功后请保持此窗口打开；断开系统分流时窗口会自动关闭。\n");

    let (mut notify_reader, notify_writer) = UnixStream::pair()?;
    notify_reader.set_read_timeout(Some(AGENT_REGISTRATION_TIMEOUT))?;
    clear_close_on_exec(notify_writer.as_raw_fd())?;
    let process = std::process::id().to_string();
    let notify_fd = notify_writer.as_raw_fd().to_string();
    let mut agent = Command::new("pkttyagent")
        .arg("--process")
        .arg(process)
        .arg("--notify-fd")
        .arg(notify_fd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    drop(notify_writer);

    let mut registered = [0_u8; 1];
    match notify_reader.read(&mut registered) {
        Ok(0) => {}
        Ok(_) => {}
        Err(error) => {
            terminate_agent(&mut agent);
            return Err(io::Error::new(
                error.kind(),
                format!("failed to register terminal Polkit agent: {error}"),
            ));
        }
    }

    let status = Command::new("pkexec")
        .arg("--disable-internal-agent")
        .arg(helper)
        .args(helper_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status();
    terminate_agent(&mut agent);
    status
}

fn clear_close_on_exec(file_descriptor: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(file_descriptor, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(file_descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn terminate_agent(agent: &mut Child) {
    let _ = agent.kill();
    let _ = agent.wait();
}

pub(crate) fn authorization_flag() -> &'static str {
    AUTHORIZATION_FLAG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_flag_is_not_a_helper_argument() {
        assert_eq!(authorization_flag(), "--authorize-helper");
    }
}
