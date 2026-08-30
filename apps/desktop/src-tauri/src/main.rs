fn main() {
    #[cfg(target_os = "linux")]
    if let Some(exit_code) = feilian_desktop_lib::run_authorization_cli() {
        std::process::exit(exit_code);
    }
    feilian_desktop_lib::run();
}
