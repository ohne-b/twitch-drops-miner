fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_request",
            "cancel_request",
            "quit_app",
            "restart_app",
            "open_app_folder",
            "desktop_settings",
            "desktop_update",
            "state_open",
            "state_next",
            "state_close",
            "smoke_result",
        ]),
    ))
    .expect("desktop build configuration");
}
