fn main() {
    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "gnu" {
        let windows = tauri_build::WindowsAttributes::new().window_icon_path("");
        attributes = attributes.windows_attributes(windows);
    }
    tauri_build::try_build(attributes).expect("failed to run tauri-build script");
}
