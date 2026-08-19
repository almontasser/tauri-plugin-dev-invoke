//! Invoke Tauri commands from an ordinary browser during development.
//!
//! `tauri-plugin-dev-invoke` starts a small HTTP server next to your app and routes every
//! request through Tauri's real IPC, so commands run exactly as they do in the webview:
//! all extractors work ([`tauri::State`], [`tauri::AppHandle`], [`tauri::Window`], ...),
//! capabilities and permissions are enforced, and plugin commands behave normally.
//!
//! It also bridges the *other* direction. Rust pushes values to the frontend by evaluating
//! `runCallback` inside a webview, which a browser tab can never receive. The plugin injects
//! a small script into the host webview that relays those callbacks back to the dev server,
//! which streams them to the browser. That makes [`tauri::ipc::Channel`], the event system
//! and plugin listeners work in the browser too.
//!
//! # Example
//!
//! ```rust,ignore
//! tauri::Builder::default()
//!     .plugin(tauri_plugin_dev_invoke::init())
//!     .run(tauri::generate_context!())
//!     .expect("error while running tauri application");
//! ```
//!
//! Use [`Builder`] to change the port, restrict which browser origins may connect, or pick
//! the window that browser requests impersonate:
//!
//! ```rust,ignore
//! tauri::Builder::default()
//!     .plugin(
//!         tauri_plugin_dev_invoke::Builder::new()
//!             .port(4000)
//!             .window("main")
//!             .allow_origin("http://localhost:5173")
//!             .build(),
//!     )
//!     .run(tauri::generate_context!())
//!     .expect("error while running tauri application");
//! ```
//!
//! Every one of those can also be set at launch, which is how you run several dev instances
//! of the same binary side by side:
//!
//! ```bash
//! DEV_INVOKE_PORT=3031 npm run tauri dev
//! ```
//!
//! | Variable | Overrides |
//! | --- | --- |
//! | `DEV_INVOKE_HOST` | [`Builder::host`] — an IP address, or `localhost` |
//! | `DEV_INVOKE_PORT` | [`Builder::port`] |
//! | `DEV_INVOKE_ALLOWED_ORIGINS` | [`Builder::allowed_origins`] — comma-separated, or `*` for any |
//!
//! The environment wins over the builder, so a hard-coded `.port()` can still be overridden
//! at launch. A value that does not parse is reported on stderr and ignored.
//!
//! # Security
//!
//! The server only starts in debug builds. In a release build it is compiled out entirely
//! unless you enable the `allow-in-release` cargo feature *and* call
//! [`Builder::enabled_in_release`].
//!
//! It binds to `127.0.0.1` and, by default, only answers browser requests coming from your
//! `build.devUrl` origin. Anything reachable through the server can run any command your
//! app exposes, so keep it on loopback and keep the origin list tight.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(any(debug_assertions, feature = "allow-in-release"))]
mod server;

use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};

use tauri::{
    plugin::{Builder as PluginBuilder, TauriPlugin},
    Runtime,
};

/// Default TCP port the dev server listens on.
pub const DEFAULT_PORT: u16 = 3030;

/// Event the host webview uses to hand callbacks back to the plugin when it cannot reach
/// the dev server over HTTP (for instance because of a restrictive CSP).
#[cfg(any(debug_assertions, feature = "allow-in-release"))]
pub(crate) const BRIDGE_EVENT: &str = "dev-invoke://callback";

/// Which browser origins are allowed to talk to the dev server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginPolicy {
    /// Allow the `build.devUrl` origin, plus any origin added with
    /// [`Builder::allow_origin`]. Falls back to [`OriginPolicy::Any`] (with a warning) when
    /// the app does not configure a `devUrl`.
    Auto,
    /// Allow every origin. Convenient, but any web page you visit while the app is running
    /// can then drive your app's commands.
    Any,
    /// Allow exactly these origins.
    List(Vec<String>),
}

/// Runtime configuration of the dev server. Build one with [`Builder`].
#[derive(Debug, Clone)]
pub struct Config {
    /// Address the server binds to. Defaults to `127.0.0.1`.
    pub host: IpAddr,
    /// Port the server binds to. Defaults to [`DEFAULT_PORT`].
    pub port: u16,
    /// Label of the webview browser requests are routed through. Defaults to `main`, then
    /// to whichever webview exists.
    pub window: Option<String>,
    /// Browser origins allowed to reach the server.
    pub origins: OriginPolicy,
    /// Extra origins to allow on top of [`OriginPolicy::Auto`].
    pub extra_origins: Vec<String>,
    /// Serve files for `convertFileSrc()` over HTTP. Defaults to `true`.
    pub serve_assets: bool,
    /// How long a single command may run before the request is answered with an error.
    pub timeout: Duration,
    /// Run the server in release builds. Requires the `allow-in-release` cargo feature.
    pub enabled_in_release: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            window: None,
            origins: OriginPolicy::Auto,
            extra_origins: Vec::new(),
            serve_assets: true,
            timeout: Duration::from_secs(600),
            enabled_in_release: false,
        }
    }
}

/// Configures the plugin.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    config: Config,
}

impl Builder {
    /// Creates a builder with the default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the address the server binds to.
    ///
    /// Only change this if you need to reach the app from another device; the server has no
    /// authentication, so anything that can connect can run your commands.
    ///
    /// Overridden by the `DEV_INVOKE_HOST` environment variable.
    pub fn host(mut self, host: impl Into<IpAddr>) -> Self {
        self.config.host = host.into();
        self
    }

    /// Sets the port the server binds to. Defaults to [`DEFAULT_PORT`].
    ///
    /// Overridden by the `DEV_INVOKE_PORT` environment variable, which is how you give each
    /// of several dev instances its own listener without rebuilding.
    pub fn port(mut self, port: u16) -> Self {
        self.config.port = port;
        self
    }

    /// Routes browser requests through the webview with this label.
    ///
    /// Browser clients impersonate a single webview: commands taking a [`tauri::Window`] or
    /// [`tauri::Webview`] receive it, and its capabilities decide what is allowed.
    pub fn window(mut self, label: impl Into<String>) -> Self {
        self.config.window = Some(label.into());
        self
    }

    /// Allows an additional browser origin, e.g. `http://localhost:5173`.
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.config.extra_origins.push(origin.into());
        self
    }

    /// Replaces the origin allowlist with exactly these origins.
    ///
    /// Overridden by the `DEV_INVOKE_ALLOWED_ORIGINS` environment variable, a comma-separated
    /// list (or `*` for any origin). Origins added with [`Builder::allow_origin`] still apply
    /// on top of whichever list wins.
    pub fn allowed_origins<I, S>(mut self, origins: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.origins = OriginPolicy::List(origins.into_iter().map(Into::into).collect());
        self
    }

    /// Accepts requests from any origin.
    ///
    /// Every page you visit while the app runs can then invoke your app's commands. Prefer
    /// [`Builder::allow_origin`].
    pub fn allow_any_origin(mut self) -> Self {
        self.config.origins = OriginPolicy::Any;
        self
    }

    /// Serves files for `convertFileSrc()` over HTTP. Enabled by default.
    ///
    /// The endpoint reads arbitrary paths off disk, which is what the `asset:` protocol does
    /// inside the webview. Disable it if you would rather not expose that on loopback.
    pub fn serve_assets(mut self, enabled: bool) -> Self {
        self.config.serve_assets = enabled;
        self
    }

    /// Sets how long a single command may run before the request fails. Defaults to 10
    /// minutes.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Runs the server in release builds too.
    ///
    /// This has no effect unless the `allow-in-release` cargo feature is enabled; without it
    /// the server is not compiled into release builds at all.
    pub fn enabled_in_release(mut self, enabled: bool) -> Self {
        self.config.enabled_in_release = enabled;
        self
    }

    /// Builds the plugin.
    pub fn build<R: Runtime>(self) -> TauriPlugin<R> {
        #[cfg(any(debug_assertions, feature = "allow-in-release"))]
        {
            if cfg!(debug_assertions) || self.config.enabled_in_release {
                return server::plugin(self.config);
            }
        }

        let _ = self;
        PluginBuilder::new("dev-invoke").build()
    }
}

/// Initializes the plugin with the default configuration.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new().build()
}
