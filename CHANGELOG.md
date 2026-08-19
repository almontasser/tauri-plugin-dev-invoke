# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). The Rust crate
and the npm package share a version number.

## [Unreleased]

## [0.3.1] - 2026-08-19

### Added

- **Headless mode** via `Builder::headless` or `DEV_INVOKE_HEADLESS`, for working entirely in
  a browser tab. The app's windows are hidden as they are created; they keep loading the
  frontend and relaying events and channels, since that is what the browser depends on. On
  macOS the app also drops out of the Dock and stops taking focus.

### Changed

- The API package's lockfile is tracked and the release script installs with `npm ci`, so a
  published build is compiled against pinned dependencies.
- `scripts/release.sh` now publishes before it pushes, so a failed publish can no longer leave
  a tag on GitHub for a release that does not exist. It also preflights the branch, working
  tree, tag and credentials, requires a non-empty `## [Unreleased]` section and rolls it into
  the new version, verifies every version rewrite actually applied, and takes `--dry-run`.

## [0.3.0] - 2026-08-19

Browser sessions now reach the whole Tauri IPC surface, not just `invoke()`.

### Added

- **Events.** `listen()`, `once()`, `emit()` and `emitTo()` work from the browser, including
  events emitted from Rust. Delivery keeps Tauri's own target filtering.
- **Channels.** `Channel` messages reach the browser in order, including payloads large enough
  to take Tauri's fetch path and binary payloads.
- **The rest of `__TAURI_INTERNALS__`.** `transformCallback`, `unregisterCallback`,
  `runCallback`, `callbacks`, `convertFileSrc`, `metadata` and `plugins.path`, plus
  `__TAURI_EVENT_PLUGIN_INTERNALS__`. `getCurrentWindow()`, `sep()` and friends now work.
- **`convertFileSrc()`** is served over HTTP with `Range` support, so `<video>` and `<audio>`
  can seek. Disable with `Builder::serve_assets(false)`.
- **Configuration** through `Builder`: `host`, `port`, `window`, `allow_origin`,
  `allowed_origins`, `allow_any_origin`, `serve_assets`, `timeout` and `enabled_in_release`.
- **Environment overrides** for `DEV_INVOKE_HOST`, `DEV_INVOKE_PORT` and
  `DEV_INVOKE_ALLOWED_ORIGINS`, so several dev instances can run side by side without a
  rebuild. The environment wins over the builder. ([#1])
- **Endpoints** beyond invoke: `GET /events` (Server-Sent Events), `GET /metadata`,
  `GET /asset/<protocol>/<path>` and `POST /callback`.
- **`allow-in-release` cargo feature**, required before `Builder::enabled_in_release` can
  start the server outside a debug build.

### Changed

- **`setupDevInvoke()` returns a promise.** It still installs synchronously, so existing
  callers keep working, but awaiting it means the window label and path separator are correct
  on the first render. Pass `{ window: "main" }` instead if you would rather not await.
- **Invokes moved to `POST /invoke/<command>`**, mirroring Tauri's own IPC: the body carries
  the arguments and a `Tauri-Response: ok|error` header distinguishes a resolved command from
  a rejected one. `POST /` and `POST /invoke` still accept the v0.2 `{ cmd, args }` shape.
- **Rejected commands return their real error value** instead of a `{:?}`-formatted string.
- **Requests are handled concurrently.** v0.2 served one request at a time.
- **The origin allowlist defaults to the `build.devUrl` origin.** v0.2 answered every origin,
  which let any page visited while the app was running drive its commands. Opt back out with
  `Builder::allow_any_origin` or `DEV_INVOKE_ALLOWED_ORIGINS=*`.
- The npm package is now dual licensed `MIT OR Apache-2.0`, matching the crate.

### Fixed

- **Binary payloads survive the round trip.** `ArrayBuffer` arguments and
  `tauri::ipc::Response` bodies stay bytes instead of being converted to JSON number arrays.
- **A webview that has not opened yet no longer fails the request.** The server waits for the
  window on a cold start rather than answering `503`.
- Requests are routed to a stable webview (the configured label, then `main`) instead of an
  arbitrary entry in a hash map.

### Known limitations

- The app window must be open: it relays Rust-to-JavaScript callbacks to the browser.
- `withGlobalTauri` is not shimmed; import from `@tauri-apps/api`.
- Window-level events (`tauri://resize`, `tauri://focus`, drag and drop) describe the app
  window, not the browser tab.
- Custom URI scheme protocols other than `asset:` are not proxied.

## [0.2.0] - 2025

### Changed

- Simplified the plugin by removing the custom command macros and routing HTTP invokes
  directly through Tauri's native handler, so command extractors behave normally.

### Added

- Release script automating version bumps, git tagging and package publishing.

## [0.1.0] - 2025

- Initial release: invoke Tauri commands over HTTP from an external browser.

[#1]: https://github.com/almontasser/tauri-plugin-dev-invoke/issues/1
[Unreleased]: https://github.com/almontasser/tauri-plugin-dev-invoke/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/almontasser/tauri-plugin-dev-invoke/releases/tag/v0.3.1
[0.3.0]: https://github.com/almontasser/tauri-plugin-dev-invoke/releases/tag/v0.3.0
[0.2.0]: https://github.com/almontasser/tauri-plugin-dev-invoke/releases/tag/v0.2.0
[0.1.0]: https://github.com/almontasser/tauri-plugin-dev-invoke/releases/tag/v0.1.0
