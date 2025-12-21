# Tauri Dev Invoke Plugin

**Unlock the power of external browsers for your Tauri development workflow.**

`tauri-plugin-dev-invoke` bridges the gap between your Tauri Rust backend and external browsers (Chrome, Firefox, Edge, Safari, mobile browsers) during development. use the standard `invoke` API you know and love, even outside the Tauri WebView.

![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![Tauri](https://img.shields.io/badge/Tauri-v2-orange)

## 🚀 Why?

By default, Tauri's secure IPC (`invoke`) only works inside the specific WebView provided by the OS. This makes it impossible to:
- Test your app in other browsers.
- Use mobile testing tools.
- Debug layout issues on different engines.

**This plugin changes that.** It spins up a secure, local HTTP server (development only) that mirrors your commands, allowing any browser to communicate with your Rust backend seamlessly.

## 📦 Installation

This plugin requires both a Rust crate and a JavaScript adapter.

### 1. Rust

Add the plugin to your `src-tauri/Cargo.toml`:

```toml
[dependencies]
# Use the correct version or path
tauri-plugin-dev-invoke = "0.1" 
```

### 2. JavaScript / TypeScript

Add the API package to your frontend dependencies:

```bash
# npm
npm install tauri-plugin-dev-invoke-api

# bun
bun add tauri-plugin-dev-invoke-api

# pnpm
pnpm add tauri-plugin-dev-invoke-api
```

## 🛠️ Usage

### Rust Setup

1.  **Replace `#[tauri::command]`**: Use `#[dev_invoke::command]` for any command you want to expose to external browsers.
2.  **Register the Plugin**: Initialize the plugin in your `lib.rs` builder.

```rust
// src-tauri/src/lib.rs
use tauri_plugin_dev_invoke::{command, dev_invoke_handler};

// 1. Use the plugin's command macro
#[command] 
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 2. Register the plugin with your commands
        .plugin(tauri_plugin_dev_invoke::init(dev_invoke_handler![greet]))
        // Standard Tauri handler registration is still required!
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### Frontend Setup

Call `setupDevInvoke()` **once** at the very start of your application (e.g., in `main.tsx` or `main.ts`).

```typescript
// src/main.tsx
import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

// This transparently patches the internal invoke mechanism
// ONLY in external browsers. Has no effect in Tauri WebView.
setupDevInvoke();

import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
```

### That's it!

Now you can use `invoke` anywhere, just as you normally would:

```typescript
import { invoke } from "@tauri-apps/api/core";

// Works in Tauri App AND in Chrome/Firefox/Safari via http://localhost:1420
const response = await invoke("greet", { name: "Developer" });
console.log(response); // "Hello, Developer!"
```

## 🔒 Security Note

**This plugin is strictly for DEVELOPMENT use.**

- The underlying HTTP server is wrapped in `#[cfg(debug_assertions)]`.
- It will **not** be included in your release builds.
- Ensure you do not manually enable it in production environments.

## 📂 Project Structure

- `packages/tauri-plugin-dev-invoke`: Main Rust crate.
- `packages/tauri-plugin-dev-invoke-macros`: Proc-macro crate.
- `packages/tauri-plugin-dev-invoke-api`: Utility library for frontend.
- `example/`: Getting started example application.

## 📝 License

MIT or Apache-2.0
