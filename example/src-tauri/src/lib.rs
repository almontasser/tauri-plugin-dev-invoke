use std::{thread, time::Duration};

use tauri::{ipc::Channel, AppHandle, Emitter, Webview};

#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {name}! You've been greeted from Rust!")
}

/// Shows that `Window`/`Webview` extractors work: a browser client impersonates a real
/// webview, so this reports the label it was routed through.
#[tauri::command]
fn whoami(webview: Webview) -> String {
    webview.label().to_string()
}

/// Streams values back over an IPC channel. The plugin relays them to the browser.
#[tauri::command]
fn count_to(limit: u32, progress: Channel<u32>) {
    thread::spawn(move || {
        for value in 1..=limit {
            if progress.send(value).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(300));
        }
    });
}

/// Emits a Tauri event from Rust, which browser listeners receive like any webview would.
#[tauri::command]
fn broadcast(app: AppHandle, message: String) -> tauri::Result<()> {
    app.emit("demo://announcement", message)
}

/// Returns raw bytes, exercising the binary response path.
#[tauri::command]
fn random_bytes(count: usize) -> tauri::ipc::Response {
    let bytes: Vec<u8> = (0..count).map(|i| (i * 37 % 251) as u8).collect();
    tauri::ipc::Response::new(bytes)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dev_invoke::init())
        .invoke_handler(tauri::generate_handler![
            greet,
            whoami,
            count_to,
            broadcast,
            random_bytes
        ])
        .setup(|app| {
            // A steady heartbeat so browser event listeners have something to show without
            // any interaction.
            let handle = app.handle().clone();
            thread::spawn(move || {
                let mut tick: u64 = 0;
                loop {
                    thread::sleep(Duration::from_secs(3));
                    tick += 1;
                    let _ = handle.emit("demo://tick", tick);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
