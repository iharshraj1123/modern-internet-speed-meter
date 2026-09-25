// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "windows")]
    let _instance_lock = unsafe {
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;

        let name: Vec<u16> = "Local\\InternetSpeedMeter_SingleInstance_Mutex\0"
            .encode_utf16()
            .collect();

        let handle = CreateMutexW(None, true, windows::core::PCWSTR(name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        handle
    };

    tauri_app_lib::run();
}
