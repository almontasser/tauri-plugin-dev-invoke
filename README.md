# Tauri Dev Invoke Plugin

**Invoke Tauri commands from external browsers** - identical behavior to WebView.

`tauri-plugin-dev-invoke` routes HTTP requests through Tauri's real invoke system, so **all extractors work** (`State`, `AppHandle`, `Window`, etc.).

![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![Tauri](https://img.shields.io/badge/Tauri-v2-orange)

## Installation

### Rust

```toml
[dependencies]
tauri-plugin-dev-invoke = "0.2"
```

### JavaScript / TypeScript

```bash
bun add tauri-plugin-dev-invoke-api
```

## Usage

### Rust Setup

Just add the plugin - **no extra configuration needed!**

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

### Frontend Setup

```typescript
import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

setupDevInvoke();  // Call once at startup

// Then use invoke normally
import { invoke } from "@tauri-apps/api/core";
const response = await invoke("greet", { name: "World" });
```

## Security

- HTTP server only runs in **debug mode** (`#[cfg(debug_assertions)]`)
- **Not included** in release builds
- Binds to `127.0.0.1:3030` only (localhost)

## Project Structure

- `packages/tauri-plugin-dev-invoke`: Rust plugin crate
- `packages/tauri-plugin-dev-invoke-api`: Frontend utility
- `example/`: Example application

## License

MIT or Apache-2.0
