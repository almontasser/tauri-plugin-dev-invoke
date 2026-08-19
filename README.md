# Tauri Dev Invoke Plugin

**Run your Tauri app in a plain browser during development.**

`tauri-plugin-dev-invoke` puts a small HTTP server next to your app and routes it through
Tauri's real IPC. Commands, plugin commands, events, channels and `convertFileSrc()` all work
from a browser tab, so you can develop against your actual Rust backend with browser devtools,
React DevTools, responsive mode and multiple tabs.

![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)
![Tauri](https://img.shields.io/badge/Tauri-v2-orange)

## What works

| | |
| --- | --- |
| `invoke("my_command")` | Runs through `Webview::on_message`, so `State`, `AppHandle`, `Window`, `Webview` and `Request` extractors all behave normally |
| Plugin commands | `plugin:fs`, `plugin:dialog`, `plugin:store`, ... — anything registered in your app |
| Capabilities & permissions | Enforced against the window the browser impersonates, so ACL errors surface exactly as they would in the app |
| `listen()` / `once()` / `emit()` | Events flow both ways, including events emitted from Rust |
| `Channel` | Streamed messages arrive in order, including large and binary payloads |
| Binary payloads | `ArrayBuffer` arguments and `tauri::ipc::Response` bodies stay bytes instead of turning into JSON arrays |
| `getCurrentWindow()`, `sep()`, ... | Window labels and path metadata are read from the running app |
| `convertFileSrc()` | Files are served over HTTP with `Range` support, so `<video>` and `<audio>` seek |

## Installation

### Rust

```toml
[dependencies]
tauri-plugin-dev-invoke = "0.3"
```

### JavaScript / TypeScript

```bash
bun add tauri-plugin-dev-invoke-api
```

## Usage

### Rust

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

### Frontend

```typescript
import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

// No-op inside the Tauri webview, so this is safe to leave in your entry point.
await setupDevInvoke();
```

Then use Tauri as you normally would:

```typescript
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

await invoke("greet", { name: "World" });
await listen("progress", (event) => console.log(event.payload));
```

Run `tauri dev` and open your dev server URL (e.g. `http://localhost:1420`) in a browser. The
app window and the browser tab talk to the same backend.

Awaiting `setupDevInvoke()` means the window label and path separator are correct on the first
render. If you would rather not await it, pass the label instead:

```typescript
setupDevInvoke({ window: "main" });
```

## Configuration

### Rust

```rust
use std::time::Duration;

tauri::Builder::default()
    .plugin(
        tauri_plugin_dev_invoke::Builder::new()
            .port(3030)                              // default 3030
            .window("main")                          // webview browser clients impersonate
            .allow_origin("http://localhost:5173")   // extra browser origin
            .serve_assets(true)                      // convertFileSrc() over HTTP
            .timeout(Duration::from_secs(600))       // per-command deadline
            .build(),
    )
```

### Frontend

```typescript
await setupDevInvoke({
    url: "http://localhost:3030", // dev server base URL
    window: "main",               // window label to impersonate
    debug: true,                  // log connection details
    force: false,                 // patch even inside the Tauri webview
});
```

### Environment variables

Every network option can also be set at launch, so you do not have to rebuild to move the
server:

| Variable | Overrides | Example |
| --- | --- | --- |
| `DEV_INVOKE_HOST` | `Builder::host` | `0.0.0.0`, or `localhost` |
| `DEV_INVOKE_PORT` | `Builder::port` | `3031` |
| `DEV_INVOKE_ALLOWED_ORIGINS` | `Builder::allowed_origins` | `http://localhost:5173,http://192.168.1.10:1420`, or `*` |

The environment **wins over the builder**, so a hard-coded `.port(3030)` can still be
overridden at launch. Unset variables change nothing, and a value that does not parse is
reported on stderr and ignored rather than silently falling back.

### Running several instances at once

Give each instance its own listener without touching the code. Each one also needs its own
dev server port in `build.devUrl`, and the frontend has to point at the matching invoke port:

```bash
DEV_INVOKE_PORT=3031 VITE_DEV_INVOKE_URL=http://localhost:3031 npm run tauri dev
```

```typescript
await setupDevInvoke({ url: import.meta.env.VITE_DEV_INVOKE_URL });
```

### Testing from another device

The defaults are loopback-only. To reach the app from a phone on the same network you have to
open both the bind address and the origin allowlist, which drops the protections described
under [Security](#security) — do it on a network you trust:

```bash
DEV_INVOKE_HOST=0.0.0.0 \
DEV_INVOKE_ALLOWED_ORIGINS=http://192.168.1.10:1420 \
  npm run tauri dev
```

```typescript
await setupDevInvoke({ url: "http://192.168.1.10:3030" });
```

Asset URLs follow the host the browser used to reach the server, so `convertFileSrc()` keeps
working from a remote device.

The same settings are available on the builder if you would rather commit them:

```rust
tauri_plugin_dev_invoke::Builder::new()
    .host([0, 0, 0, 0])
    .allow_origin("http://192.168.1.10:1420")
    .build()
```

## How it works

Two directions, two mechanisms.

**Browser → Rust.** `setupDevInvoke()` installs a stand-in for `window.__TAURI_INTERNALS__`.
`invoke()` becomes an HTTP request that the plugin turns into a real `InvokeRequest` and hands
to `Webview::on_message` — the same entry point the webview's own IPC uses.

**Rust → Browser.** Rust pushes values to the frontend by evaluating
`__TAURI_INTERNALS__.runCallback(id, payload)` inside a webview, which a browser tab can never
receive. The plugin injects a script into your app window that relays any callback id the
window does not own back to the dev server, which streams it to the browser over Server-Sent
Events. Events, `Channel` messages and plugin listeners all ride that one mechanism, so they
keep Tauri's own target filtering and ordering.

This is why your app window has to be open: it is the relay. Callbacks are broadcast to every
connected tab and filtered client-side by id, so several browser tabs can be open at once.

## HTTP API

If you want to drive the app from something other than a browser — a test script, `curl`,
another process — the endpoints are plain HTTP.

| | |
| --- | --- |
| `POST /invoke/<command>` | Body is the arguments (`application/json`, or `application/octet-stream` for raw bytes). The `Tauri-Response: ok\|error` header says whether the command resolved or rejected, mirroring Tauri's own IPC |
| `GET /events` | Server-Sent Events stream of Rust → JS callbacks |
| `GET /metadata` | Window labels, OS, path separator, asset base URL |
| `GET /asset/asset/<url-encoded path>` | The file, with `Range` support |
| `POST /` and `POST /invoke` | `{ "cmd": ..., "args": ... }`, kept for v0.2 clients |

```bash
curl -X POST http://127.0.0.1:3030/invoke/greet \
  -H 'Content-Type: application/json' \
  -d '{"name":"curl"}'
```

## Security

The server can run any command your app exposes, so it is deliberately restricted:

- It **only starts in debug builds**. A release build does not contain it at all unless you
  enable the `allow-in-release` cargo feature *and* call `Builder::enabled_in_release(true)`.
- It binds to `127.0.0.1` only.
- By default it only answers browsers whose origin matches your `build.devUrl`. Add more with
  `Builder::allow_origin` or `DEV_INVOKE_ALLOWED_ORIGINS`, or opt out with
  `Builder::allow_any_origin` / `DEV_INVOKE_ALLOWED_ORIGINS=*` — which lets any page you visit
  while the app is running drive your commands.
- `GET /asset` reads arbitrary paths, the same as the `asset:` protocol does inside the
  webview. Turn it off with `Builder::serve_assets(false)`.

There is no authentication: anything that can open a socket to the port can invoke commands.
Keep it on loopback.

## Limitations

- Your app window must be open — it relays Rust → JS callbacks to the browser.
- `withGlobalTauri` is not shimmed; import from `@tauri-apps/api` instead of `window.__TAURI__`.
- Window-level events (`tauri://resize`, `tauri://focus`, drag and drop) describe the *app
  window*, not the browser tab, because that is the window the browser impersonates.
- Custom URI scheme protocols other than `asset:` are not proxied.

## Project structure

- `packages/tauri-plugin-dev-invoke` — Rust plugin crate
- `packages/tauri-plugin-dev-invoke-api` — browser shim
- `example/` — demo app exercising commands, events, channels and binary responses

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this project by you shall be dual licensed as above, without any additional terms or
conditions.
