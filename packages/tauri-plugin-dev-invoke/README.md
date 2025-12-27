# Tauri Dev Invoke Plugin

**Invoke Tauri commands from external browsers** - identical behavior to WebView.

Routes HTTP requests through Tauri's real invoke system, so **all extractors work**.

## Installation

```toml
[dependencies]
tauri-plugin-dev-invoke = "0.2"
```

## Usage

```rust
#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dev_invoke::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error");
}
```

## Security

- HTTP server only runs in **debug mode**
- Binds to `127.0.0.1:3030` (localhost only)

## License

MIT or Apache-2.0
