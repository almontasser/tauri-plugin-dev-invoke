# tauri-plugin-dev-invoke-api

Browser-side half of [`tauri-plugin-dev-invoke`](https://crates.io/crates/tauri-plugin-dev-invoke).

Installs a stand-in for `window.__TAURI_INTERNALS__` that speaks HTTP to the plugin's dev
server, so `@tauri-apps/api` and every Tauri plugin behave the same way in a browser tab as
they do inside the app: `invoke()`, `listen()`, `Channel`, `getCurrentWindow()`,
`convertFileSrc()` and binary payloads all work.

## Installation

```bash
npm install tauri-plugin-dev-invoke-api
```

The Rust crate has to be registered in your app for any of this to do anything:

```toml
[dependencies]
tauri-plugin-dev-invoke = "0.3"
```

## Usage

Call it once at your entry point. Inside the Tauri webview it is a no-op, so it is safe to
leave in.

```typescript
import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

await setupDevInvoke();
```

Then use Tauri as you normally would:

```typescript
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

await invoke("greet", { name: "World" });
await listen("progress", (event) => console.log(event.payload));
```

Awaiting the call means the window label and path separator are correct on the first render.
If you would rather not await it, pass the label instead: `setupDevInvoke({ window: "main" })`.

## Options

```typescript
await setupDevInvoke({
    url: "http://localhost:3030", // dev server base URL
    window: "main",               // window label to impersonate
    debug: true,                  // log connection details
    force: false,                 // patch even inside the Tauri webview
});
```

It resolves to a handle with the server `metadata` and a `teardown()` that removes the shim.
`isDevInvokeActive()` and `getDevInvokeHandle()` are also exported.

See the [repository README](https://github.com/almontasser/tauri-plugin-dev-invoke) for the
Rust-side options and how the callback bridge works.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
