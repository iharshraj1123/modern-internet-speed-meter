fn main() {
    if std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "gnu" {
        if std::env::var("WINDRES_FLAGS").is_err() {
            std::env::set_var("WINDRES_FLAGS", "-F pe-x86-64");
        }
    }
    tauri_build::build();
}
