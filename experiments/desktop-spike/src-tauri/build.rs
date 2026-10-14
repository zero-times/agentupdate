fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "run_validation",
            "finish_validation",
            "admin_only_probe",
        ]),
    ))
    .expect("Could not build Tauri manifest");
}
