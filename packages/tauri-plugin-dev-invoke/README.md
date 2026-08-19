# tauri-plugin-dev-invoke

**Run your Tauri app in a plain browser during development.**

Starts a small HTTP server next to your app and routes it through Tauri's real IPC, so
commands, plugin commands, events, channels and `convertFileSrc()` all work from a browser
tab. Command extractors (`State`, `AppHandle`, `Window`, `Webview`, `Request`) and the
capability ACL behave exactly as they do in the webview.

Pair it with the [`tauri-plugin-dev-invoke-api`](https://www.npmjs.com/package/tauri-plugin-dev-invoke-api)
npm package, which installs the browser-side shim.

## Installation

```toml
[dependencies]
tauri-plugin-dev-invoke = "0.3"
```

## Usage

```rust
#[tauri::command]
fn greet(name: String) -> String {
    format!("Hello, {name}!")
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dev_invoke::init())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

Configure it with `Builder`:

```rust
tauri_plugin_dev_invoke::Builder::new()
    .port(3030)
    .window("main")
    .allow_origin("http://localhost:5173")
    .build()
```

The network options can also be set at launch, which is how you run several dev instances of
the same binary side by side:

| Variable | Overrides |
| --- | --- |
| `DEV_INVOKE_HOST` | `Builder::host` — an IP address, or `localhost` |
| `DEV_INVOKE_PORT` | `Builder::port` |
| `DEV_INVOKE_ALLOWED_ORIGINS` | `Builder::allowed_origins` — comma-separated, or `*` for any |
| `DEV_INVOKE_HEADLESS` | `Builder::headless` — hides the app windows so it runs as a background relay |

```bash
DEV_INVOKE_PORT=3031 npm run tauri dev
```

The environment wins over the builder, so a hard-coded `.port()` can still be overridden at
launch.

## Security

- The server **only starts in debug builds**. Release builds do not contain it unless you
  enable the `allow-in-release` feature *and* call `Builder::enabled_in_release(true)`.
- It binds to `127.0.0.1` and, by default, only answers browsers whose origin matches your
  `build.devUrl`.
- Anything that reaches the port can run any command your app exposes. Keep it on loopback.

See the [repository README](https://github.com/almontasser/tauri-plugin-dev-invoke) for the
HTTP API, the full option list and how the callback bridge works.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
