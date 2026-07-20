fn main() {
    if let Err(e) = tauri_build::try_build(tauri_build::Attributes::new()) {
        eprintln!("cargo:warning=Tauri build notice: {}", e);
    }
}
