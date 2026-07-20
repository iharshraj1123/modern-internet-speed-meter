use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    if env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default() == "gnu" {
        if let Ok(out_dir) = env::var("OUT_DIR") {
            let wrapper_path = PathBuf::from(&out_dir).join("windres_wrapper.cmd");
            let content = "@echo off\r\nwindres -F pe-x86-64 %*\r\n";
            if fs::write(&wrapper_path, content).is_ok() {
                env::set_var("WINDRES", wrapper_path.to_str().unwrap());
            }
        }
    }
    tauri_build::build();
}
