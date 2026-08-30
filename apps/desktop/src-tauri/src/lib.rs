mod controller;
#[cfg(target_os = "linux")]
mod display_backend;
mod launcher;
mod secret_store;

use controller::{
    AppController, AuthConfiguration, AuthSnapshot, ControllerError, HelperMode, HelperSnapshot,
};
use tauri::Manager;

#[tauri::command]
async fn helper_status(
    controller: tauri::State<'_, AppController>,
    mode: HelperMode,
) -> Result<HelperSnapshot, String> {
    Ok(controller.status(mode).await)
}

#[tauri::command]
async fn helper_connect(
    controller: tauri::State<'_, AppController>,
    mode: HelperMode,
    node_id: i32,
) -> Result<HelperSnapshot, ControllerError> {
    controller.connect(mode, node_id).await
}

#[tauri::command]
async fn helper_stop(
    controller: tauri::State<'_, AppController>,
    mode: HelperMode,
) -> Result<HelperSnapshot, String> {
    Ok(controller.stop(mode).await)
}

#[tauri::command]
async fn helper_cleanup(
    controller: tauri::State<'_, AppController>,
    mode: HelperMode,
) -> Result<HelperSnapshot, String> {
    Ok(controller.cleanup(mode).await)
}

#[tauri::command]
async fn auth_status(
    controller: tauri::State<'_, AppController>,
) -> Result<AuthSnapshot, ControllerError> {
    controller.auth_status().await
}

#[tauri::command]
async fn auth_configure(
    controller: tauri::State<'_, AppController>,
    configuration: AuthConfiguration,
) -> Result<AuthSnapshot, ControllerError> {
    controller.auth_configure(configuration).await
}

#[tauri::command]
async fn auth_begin_qr(
    controller: tauri::State<'_, AppController>,
) -> Result<AuthSnapshot, ControllerError> {
    controller.auth_begin_qr().await
}

#[tauri::command]
async fn auth_poll_qr(
    controller: tauri::State<'_, AppController>,
) -> Result<AuthSnapshot, ControllerError> {
    controller.auth_poll_qr().await
}

#[tauri::command]
async fn auth_refresh_nodes(
    controller: tauri::State<'_, AppController>,
) -> Result<AuthSnapshot, ControllerError> {
    controller.auth_refresh_nodes().await
}

#[tauri::command]
async fn auth_reset(
    controller: tauri::State<'_, AppController>,
) -> Result<AuthSnapshot, ControllerError> {
    controller.auth_reset().await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(target_os = "linux")]
    display_backend::configure();

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            let controller = AppController::new(data_dir);
            let _ = tauri::async_runtime::block_on(controller.initialize());
            app.manage(controller);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            helper_status,
            helper_connect,
            helper_stop,
            helper_cleanup,
            auth_status,
            auth_configure,
            auth_begin_qr,
            auth_poll_qr,
            auth_refresh_nodes,
            auth_reset
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Feilian Lite desktop application");
}
