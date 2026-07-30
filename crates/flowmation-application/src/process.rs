use tokio::process::{Child, Command};

#[cfg(unix)]
pub(crate) fn configure_process_tree(command: &mut Command) {
    command.process_group(0);
}

#[cfg(windows)]
pub(crate) fn configure_process_tree(_command: &mut Command) {}

#[cfg(unix)]
pub(crate) async fn terminate_process_tree(child: &mut Child, process_id: Option<u32>) {
    if let Some(process_id) = process_id.and_then(|process_id| i32::try_from(process_id).ok()) {
        let _result = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(process_id),
            nix::sys::signal::Signal::SIGTERM,
        );
    }
    let _result = child.kill().await;
    let _result = child.wait().await;
}

#[cfg(windows)]
pub(crate) async fn terminate_process_tree(child: &mut Child, process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        let _result = Command::new("taskkill.exe")
            .args(["/pid", &process_id.to_string(), "/t", "/f"])
            .status()
            .await;
    }
    let _result = child.kill().await;
    let _result = child.wait().await;
}
