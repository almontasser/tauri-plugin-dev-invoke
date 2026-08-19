//! The HTTP server browser clients talk to, and the plumbing that makes a browser tab look
//! like a Tauri webview.
//!
//! Requests are turned into real [`InvokeRequest`]s and handed to
//! [`Webview::on_message`], which is the same entry point the webview's own IPC uses. That
//! keeps command extractors, capabilities and plugin routing identical to the real thing.
//!
//! The reverse direction is handled by `bridge.js`: values Rust pushes to the frontend are
//! relayed out of the host webview, arrive here through [`handle_callback`] (or the
//! [`crate::BRIDGE_EVENT`] fallback) and are streamed to browsers over Server-Sent Events.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{
    http::{HeaderMap, HeaderName, HeaderValue},
    ipc::{CallbackFn, InvokeBody, InvokeResponse, InvokeResponseBody},
    plugin::{Builder as PluginBuilder, TauriPlugin},
    webview::InvokeRequest,
    AppHandle, Listener, Manager, Runtime, Url, Webview,
};
use tiny_http::{Header, Method, Request, Response, Server as HttpServer, StatusCode};

use crate::{Config, OriginPolicy, BRIDGE_EVENT};

/// How long a request waits for a webview to exist before giving up. The server comes up
/// before the window on a cold start.
const WEBVIEW_WAIT: Duration = Duration::from_secs(10);

/// Interval between SSE keep-alive comments.
const SSE_PING: Duration = Duration::from_secs(15);

/// Request headers that describe the HTTP hop itself and must not be forwarded into the
/// command's [`tauri::ipc::Request`].
const SKIPPED_HEADERS: &[&str] = &[
    "connection",
    "content-length",
    "host",
    "origin",
    "referer",
    "transfer-encoding",
    "upgrade",
    "user-agent",
];

/// Header a client uses to pick which webview its request impersonates.
const WINDOW_HEADER: &str = "x-dev-invoke-window";
/// Header carrying the client id, for logging only.
const CLIENT_HEADER: &str = "x-dev-invoke-client";
/// Mirrors Tauri's own IPC: tells the client whether the promise resolved or rejected.
const RESPONSE_HEADER: &str = "Tauri-Response";

// ---------------------------------------------------------------------------------------
// plugin wiring
// ---------------------------------------------------------------------------------------

pub(crate) fn plugin<R: Runtime>(mut config: Config) -> TauriPlugin<R> {
    apply_env_overrides(&mut config, |name| std::env::var(name).ok());

    let endpoint = format!("{}/callback", base_url(&config));
    let script = include_str!("bridge.js")
        .replace("__TEMPLATE_endpoint__", &endpoint)
        .replace("__TEMPLATE_bridge_event__", BRIDGE_EVENT);

    PluginBuilder::new("dev-invoke")
        .js_init_script(script)
        .setup(move |app, _api| {
            let state = Inner::new(app.clone(), config);

            // Fallback transport: the host webview emits this when a CSP keeps it from
            // reaching the server with `fetch`.
            let listener_state = state.clone();
            app.listen_any(BRIDGE_EVENT, move |event| {
                match serde_json::from_str::<Vec<CallbackMessage>>(event.payload()) {
                    Ok(messages) => listener_state.dispatch(&messages),
                    Err(e) => eprintln!("[dev-invoke] discarding malformed bridge payload: {e}"),
                }
            });

            thread::spawn(move || run(state));
            Ok(())
        })
        .build()
}

// ---------------------------------------------------------------------------------------
// environment overrides
// ---------------------------------------------------------------------------------------

/// Overrides the bind address, e.g. `0.0.0.0`. `localhost` is accepted too.
const HOST_ENV: &str = "DEV_INVOKE_HOST";
/// Overrides the port.
const PORT_ENV: &str = "DEV_INVOKE_PORT";
/// Replaces the origin allowlist with a comma-separated list, or `*` for any origin.
const ORIGINS_ENV: &str = "DEV_INVOKE_ALLOWED_ORIGINS";

/// Applies the `DEV_INVOKE_*` variables on top of the builder's values.
///
/// The environment deliberately wins over the builder: setting a variable is an explicit act,
/// and overriding a hard-coded `.port()` at launch is the only way to run several dev
/// instances of the same binary side by side. Unset variables change nothing, and a value
/// that does not parse is reported and ignored rather than falling back silently.
///
/// `var` is injected so the parsing can be tested without touching the process environment.
fn apply_env_overrides(config: &mut Config, var: impl Fn(&str) -> Option<String>) {
    if let Some(raw) = var(HOST_ENV) {
        let value = raw.trim();
        match parse_host(value) {
            Some(host) => config.host = host,
            None => ignored(
                HOST_ENV,
                value,
                "expected an IP address, e.g. 127.0.0.1 or 0.0.0.0",
            ),
        }
    }

    if let Some(raw) = var(PORT_ENV) {
        let value = raw.trim();
        match value.parse::<u16>() {
            // The bridge script is baked with the server's address before the socket is
            // bound, so an OS-assigned port would leave the app window unable to call back.
            Ok(0) => ignored(PORT_ENV, value, "the port must be fixed, not OS-assigned"),
            Ok(port) => config.port = port,
            Err(_) => ignored(PORT_ENV, value, "expected a port number"),
        }
    }

    if let Some(raw) = var(ORIGINS_ENV) {
        let value = raw.trim();
        let origins: Vec<String> = value
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect();

        if value == "*" {
            config.origins = OriginPolicy::Any;
        } else if origins.is_empty() {
            ignored(
                ORIGINS_ENV,
                value,
                "expected a comma-separated list of origins, or `*` for any",
            );
        } else {
            config.origins = OriginPolicy::List(origins);
        }
    }
}

fn ignored(name: &str, value: &str, reason: &str) {
    eprintln!("[dev-invoke] ignoring {name}={value:?}: {reason}");
}

fn parse_host(value: &str) -> Option<IpAddr> {
    match value {
        // The server binds an address rather than a name, but this is the spelling people
        // reach for.
        "localhost" => Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        other => other.parse().ok(),
    }
}

// ---------------------------------------------------------------------------------------
// shared state
// ---------------------------------------------------------------------------------------

struct Inner<R: Runtime> {
    app: AppHandle<R>,
    invoke_key: String,
    config: Config,
    /// `None` means every origin is allowed.
    allowed_origins: Option<Vec<String>>,
    base_url: String,
    clients: Mutex<HashMap<u64, Sender<Vec<u8>>>>,
    next_client_id: AtomicU64,
}

type State<R> = Arc<Inner<R>>;

impl<R: Runtime> Inner<R> {
    fn new(app: AppHandle<R>, config: Config) -> State<R> {
        let allowed_origins = resolve_origins(&app, &config);
        Arc::new(Inner {
            invoke_key: app.invoke_key().to_string(),
            base_url: base_url(&config),
            app,
            config,
            allowed_origins,
            clients: Mutex::new(HashMap::new()),
            next_client_id: AtomicU64::new(1),
        })
    }

    fn origin_allowed(&self, origin: &str) -> bool {
        match &self.allowed_origins {
            None => true,
            Some(list) => list.iter().any(|allowed| allowed == origin),
        }
    }

    fn register_client(&self, tx: Sender<Vec<u8>>) -> u64 {
        let id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
        self.clients.lock().unwrap().insert(id, tx);
        id
    }

    fn unregister_client(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    /// Fans callbacks out to every connected browser.
    ///
    /// Callback ids are random 32-bit values minted per client, so a client can safely
    /// ignore ids it did not create. Broadcasting avoids having to track ownership (and the
    /// races that come with it) at the cost of some redundant traffic when several tabs are
    /// open on the same app.
    fn dispatch(&self, messages: &[CallbackMessage]) {
        if messages.is_empty() {
            return;
        }

        let mut frames = Vec::with_capacity(messages.len());
        for message in messages {
            match serde_json::to_string(message) {
                Ok(json) => frames.push(format!("event: callback\ndata: {json}\n\n").into_bytes()),
                Err(e) => eprintln!("[dev-invoke] could not serialize callback: {e}"),
            }
        }

        let mut clients = self.clients.lock().unwrap();
        clients.retain(|_, tx| frames.iter().all(|frame| tx.send(frame.clone()).is_ok()));
    }
}

fn base_url(config: &Config) -> String {
    let host = match config.host {
        IpAddr::V4(v4) if v4.is_unspecified() => "127.0.0.1".to_string(),
        IpAddr::V6(v6) if v6.is_unspecified() => "[::1]".to_string(),
        IpAddr::V6(v6) => format!("[{v6}]"),
        IpAddr::V4(v4) => v4.to_string(),
    };
    format!("http://{host}:{}", config.port)
}

/// Builds the origin allowlist. `None` means "allow anything".
fn resolve_origins<R: Runtime>(app: &AppHandle<R>, config: &Config) -> Option<Vec<String>> {
    let mut origins = match &config.origins {
        OriginPolicy::Any => return None,
        OriginPolicy::List(list) => list.clone(),
        OriginPolicy::Auto => match app.config().build.dev_url.as_ref().map(origin_of) {
            Some(dev_origin) => vec![dev_origin],
            None => {
                eprintln!(
                    "[dev-invoke] no `build.devUrl` in tauri.conf.json, so every origin is \
                     allowed. Restrict this with `Builder::allow_origin`."
                );
                return None;
            }
        },
    };

    origins.extend(config.extra_origins.iter().cloned());
    origins.sort();
    origins.dedup();
    Some(origins)
}

fn origin_of(url: &Url) -> String {
    let mut origin = format!(
        "{}://{}",
        url.scheme(),
        url.host_str().unwrap_or("localhost")
    );
    if let Some(port) = url.port() {
        origin.push_str(&format!(":{port}"));
    }
    origin
}

// ---------------------------------------------------------------------------------------
// server loop and routing
// ---------------------------------------------------------------------------------------

fn run<R: Runtime>(state: State<R>) {
    let addr = SocketAddr::new(state.config.host, state.config.port);
    let server = match HttpServer::http(addr) {
        Ok(server) => Arc::new(server),
        Err(e) => {
            eprintln!("[dev-invoke] could not bind {addr}: {e}");
            return;
        }
    };

    println!("[dev-invoke] serving Tauri IPC on {}", state.base_url);
    match &state.allowed_origins {
        Some(origins) => println!("[dev-invoke] allowed origins: {}", origins.join(", ")),
        None => println!("[dev-invoke] allowed origins: any"),
    }

    loop {
        match server.recv() {
            Ok(request) => {
                let state = state.clone();
                thread::spawn(move || handle(state, request));
            }
            Err(e) => {
                eprintln!("[dev-invoke] server stopped: {e}");
                break;
            }
        }
    }
}

fn handle<R: Runtime>(state: State<R>, mut request: Request) {
    let origin = header_value(&request, "origin");
    let path = request
        .url()
        .split(['?', '#'])
        .next()
        .unwrap_or("/")
        .to_string();
    let method = request.method().clone();

    // Assets are loaded by `<img>`, `<video>` and friends, which do not always send an
    // origin. They are read-only and mirror the `asset:` protocol, so they skip the check.
    let is_asset = path == "/asset" || path.starts_with("/asset/");

    if let Some(origin) = &origin {
        if !is_asset && !state.origin_allowed(origin) {
            eprintln!("[dev-invoke] rejected request from disallowed origin {origin}");
            let _ = request.respond(
                Response::from_string(format!("origin {origin} is not allowed"))
                    .with_status_code(403),
            );
            return;
        }
    }

    let cors = cors_headers(origin.as_deref());

    if method == Method::Options {
        let mut response = Response::empty(204);
        for header in cors {
            response.add_header(header);
        }
        let _ = request.respond(response);
        return;
    }

    match (&method, path.as_str()) {
        (Method::Get, "/") => respond(request, info(&state), cors),
        (Method::Get, "/metadata") => {
            let host = header_value(&request, "host");
            respond(request, metadata(&state, host.as_deref()), cors)
        }
        (Method::Get, "/events") => handle_events(&state, request, cors),
        (Method::Post, "/callback") => {
            let response = handle_callback(&state, &mut request);
            respond(request, response, cors)
        }
        (Method::Post, "/") | (Method::Post, "/invoke") => {
            let response = handle_legacy_invoke(&state, &mut request);
            respond(request, response, cors)
        }
        (Method::Post, path) if path.starts_with("/invoke/") => {
            let cmd = percent_decode(&path["/invoke/".len()..]);
            let response = handle_invoke(&state, &mut request, cmd);
            respond(request, response, cors)
        }
        (Method::Get, path) if path.starts_with("/asset/") => {
            handle_asset(&state, request, &path["/asset/".len()..], cors)
        }
        _ => respond(request, text(404, "not found"), cors),
    }
}

fn respond(request: Request, mut response: Response<Cursor<Vec<u8>>>, cors: Vec<Header>) {
    for header in cors {
        response.add_header(header);
    }
    let _ = request.respond(response);
}

fn cors_headers(origin: Option<&str>) -> Vec<Header> {
    vec![
        header("Access-Control-Allow-Origin", origin.unwrap_or("*")),
        header("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        header("Access-Control-Allow-Headers", "*"),
        header("Access-Control-Expose-Headers", RESPONSE_HEADER),
        header("Vary", "Origin"),
    ]
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("dev-invoke built an invalid header")
}

fn header_value(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

fn text(status: u16, body: &str) -> Response<Cursor<Vec<u8>>> {
    Response::from_data(body.as_bytes().to_vec())
        .with_status_code(status)
        .with_header(header("Content-Type", "text/plain; charset=utf-8"))
}

fn json(status: u16, body: Vec<u8>) -> Response<Cursor<Vec<u8>>> {
    Response::from_data(body)
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"))
}

fn json_value(status: u16, value: &impl Serialize) -> Response<Cursor<Vec<u8>>> {
    match serde_json::to_vec(value) {
        Ok(body) => json(status, body),
        Err(e) => text(500, &format!("failed to serialize response: {e}")),
    }
}

// ---------------------------------------------------------------------------------------
// GET / and GET /metadata
// ---------------------------------------------------------------------------------------

fn info<R: Runtime>(state: &State<R>) -> Response<Cursor<Vec<u8>>> {
    json_value(
        200,
        &serde_json::json!({
            "name": "tauri-plugin-dev-invoke",
            "version": env!("CARGO_PKG_VERSION"),
            "endpoints": {
                "invoke": "POST /invoke/<command>",
                "events": "GET /events",
                "metadata": "GET /metadata",
                "asset": "GET /asset/<protocol>/<path>",
            },
            "clients": state.clients.lock().unwrap().len(),
        }),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    /// Label of the window browser clients impersonate.
    window: String,
    /// Label of the webview browser clients impersonate.
    webview: String,
    /// Every webview label currently open.
    webviews: Vec<String>,
    /// `macos`, `windows`, `linux`, ...
    os: &'static str,
    path: PathMetadata,
    asset_base: String,
    serve_assets: bool,
    plugin_version: &'static str,
}

#[derive(Serialize)]
struct PathMetadata {
    sep: String,
    delimiter: &'static str,
}

/// `host` is the client's `Host` header. Asset URLs are built from it rather than from the
/// bind address, so a browser on another machine gets URLs that point back at this server
/// instead of at itself.
fn metadata<R: Runtime>(state: &State<R>, host: Option<&str>) -> Response<Cursor<Vec<u8>>> {
    let mut webviews: Vec<String> = state.app.webview_windows().into_keys().collect();
    webviews.sort();

    // Browser clients impersonate a webview window, whose window and webview share a label.
    let label = pick_webview(state)
        .map(|webview| webview.label().to_string())
        .or_else(|| state.config.window.clone())
        .unwrap_or_else(|| "main".to_string());

    json_value(
        200,
        &Metadata {
            window: label.clone(),
            webview: label,
            webviews,
            os: os_name(),
            path: PathMetadata {
                sep: std::path::MAIN_SEPARATOR.to_string(),
                delimiter: if cfg!(windows) { ";" } else { ":" },
            },
            asset_base: match host {
                Some(host) => format!("http://{host}/asset"),
                None => format!("{}/asset", state.base_url),
            },
            serve_assets: state.config.serve_assets,
            plugin_version: env!("CARGO_PKG_VERSION"),
        },
    )
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" | "ios" => "macos",
        "windows" => "windows",
        other => other,
    }
}

// ---------------------------------------------------------------------------------------
// POST /invoke/<command>
// ---------------------------------------------------------------------------------------

fn handle_invoke<R: Runtime>(
    state: &State<R>,
    request: &mut Request,
    cmd: String,
) -> Response<Cursor<Vec<u8>>> {
    let body = match read_body(request) {
        Ok(body) => body,
        Err(e) => return text(400, &format!("failed to read request body: {e}")),
    };

    let headers = forwarded_headers(request);
    let content_type = headers
        .get(tauri::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let invoke_body = if content_type == "application/json" {
        if body.is_empty() {
            InvokeBody::Json(serde_json::Value::Object(Default::default()))
        } else {
            match serde_json::from_slice::<serde_json::Value>(&body) {
                Ok(value) => InvokeBody::Json(value),
                Err(e) => return text(400, &format!("invalid JSON body: {e}")),
            }
        }
    } else {
        InvokeBody::Raw(body)
    };

    let label = header_value(request, WINDOW_HEADER);
    match invoke(state, cmd, invoke_body, headers, label.as_deref()) {
        Ok(InvokeResponse::Ok(InvokeResponseBody::Json(value))) => {
            json(200, value.into_bytes()).with_header(header(RESPONSE_HEADER, "ok"))
        }
        Ok(InvokeResponse::Ok(InvokeResponseBody::Raw(bytes))) => Response::from_data(bytes)
            .with_status_code(200)
            .with_header(header("Content-Type", "application/octet-stream"))
            .with_header(header(RESPONSE_HEADER, "ok")),
        // Tauri answers a rejected command with 200 plus a header, so that transport errors
        // stay distinguishable from command errors. Mirror that.
        Ok(InvokeResponse::Err(e)) => {
            json_value(200, &e.0).with_header(header(RESPONSE_HEADER, "error"))
        }
        Err(e) => text(503, &e),
    }
}

/// The request/response shape used by v0.2 of this plugin. Kept so an older frontend keeps
/// working against a newer app.
#[derive(Deserialize)]
struct LegacyInvoke {
    cmd: String,
    #[serde(default)]
    args: serde_json::Value,
}

fn handle_legacy_invoke<R: Runtime>(
    state: &State<R>,
    request: &mut Request,
) -> Response<Cursor<Vec<u8>>> {
    let body = match read_body(request) {
        Ok(body) => body,
        Err(e) => return text(400, &format!("failed to read request body: {e}")),
    };

    let payload: LegacyInvoke = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(e) => return text(400, &format!("invalid JSON body: {e}")),
    };

    let label = header_value(request, WINDOW_HEADER);
    match invoke(
        state,
        payload.cmd,
        InvokeBody::Json(payload.args),
        HeaderMap::new(),
        label.as_deref(),
    ) {
        Ok(InvokeResponse::Ok(InvokeResponseBody::Json(value))) => json(200, value.into_bytes()),
        Ok(InvokeResponse::Ok(InvokeResponseBody::Raw(bytes))) => Response::from_data(bytes)
            .with_status_code(200)
            .with_header(header("Content-Type", "application/octet-stream")),
        Ok(InvokeResponse::Err(e)) => json_value(500, &serde_json::json!({ "error": e.0 })),
        Err(e) => json_value(503, &serde_json::json!({ "error": e })),
    }
}

/// Runs a command through Tauri's real IPC and waits for its response.
fn invoke<R: Runtime>(
    state: &State<R>,
    cmd: String,
    body: InvokeBody,
    headers: HeaderMap,
    label: Option<&str>,
) -> Result<InvokeResponse, String> {
    let webview = wait_for_webview(state, label)
        .ok_or_else(|| "no webview is available to route the request through".to_string())?;

    let url = webview
        .url()
        .unwrap_or_else(|_| Url::parse("tauri://localhost").expect("valid fallback url"));

    let request = InvokeRequest {
        cmd: cmd.clone(),
        // The HTTP response carries the result, so these ids are never evaluated. Tauri
        // still requires them to build the request.
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url,
        body,
        headers,
        invoke_key: state.invoke_key.clone(),
    };

    let (tx, rx) = mpsc::channel();
    webview.on_message(
        request,
        Box::new(move |_webview, _cmd, response, _callback, _error| {
            let _ = tx.send(response);
        }),
    );

    rx.recv_timeout(state.config.timeout)
        .map_err(|_| format!("timed out waiting for command `{cmd}`"))
}

/// Resolves the webview a request is routed through, waiting for the window to open on a
/// cold start.
fn wait_for_webview<R: Runtime>(state: &State<R>, label: Option<&str>) -> Option<Webview<R>> {
    let deadline = Instant::now() + WEBVIEW_WAIT;
    loop {
        if let Some(webview) = label
            .and_then(|label| get_webview(state, label))
            .or_else(|| pick_webview(state))
        {
            return Some(webview);
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Picks the default webview: the configured label, then `main`, then the
/// lexicographically first one so the choice stays stable across runs.
fn pick_webview<R: Runtime>(state: &State<R>) -> Option<Webview<R>> {
    if let Some(label) = &state.config.window {
        return get_webview(state, label);
    }
    if let Some(webview) = get_webview(state, "main") {
        return Some(webview);
    }
    let windows = state.app.webview_windows();
    let mut labels: Vec<&String> = windows.keys().collect();
    labels.sort();
    labels
        .first()
        .and_then(|label| windows.get(*label))
        .map(|window| window.as_ref().clone())
}

fn get_webview<R: Runtime>(state: &State<R>, label: &str) -> Option<Webview<R>> {
    state
        .app
        .get_webview_window(label)
        .map(|window| window.as_ref().clone())
}

fn read_body(request: &mut Request) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    request.as_reader().read_to_end(&mut body)?;
    Ok(body)
}

/// Copies the client's headers into the shape commands see through
/// [`tauri::ipc::Request::headers`], dropping the ones that describe the HTTP hop.
fn forwarded_headers(request: &Request) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for h in request.headers() {
        let name = h.field.as_str().as_str().to_ascii_lowercase();
        if SKIPPED_HEADERS.contains(&name.as_str())
            || name == WINDOW_HEADER
            || name == CLIENT_HEADER
            || name.starts_with("sec-")
            || name.starts_with("access-control-")
        {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(h.value.as_str()),
        ) {
            headers.insert(name, value);
        }
    }
    headers
}

// ---------------------------------------------------------------------------------------
// GET /events and POST /callback
// ---------------------------------------------------------------------------------------

/// One Rust -> JS callback on its way to a browser.
///
/// `value` is the payload as the host webview saw it, with any binary parts replaced by
/// `{"__devInvokeBinary__": "<base64>"}` markers that the browser turns back into
/// `ArrayBuffer`s. The plugin only forwards it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CallbackMessage {
    /// Callback id minted by the browser's `transformCallback`.
    id: u32,
    value: serde_json::Value,
}

fn handle_callback<R: Runtime>(
    state: &State<R>,
    request: &mut Request,
) -> Response<Cursor<Vec<u8>>> {
    let body = match read_body(request) {
        Ok(body) => body,
        Err(e) => return text(400, &format!("failed to read request body: {e}")),
    };

    match serde_json::from_slice::<Vec<CallbackMessage>>(&body) {
        Ok(messages) => {
            state.dispatch(&messages);
            Response::from_data(Vec::new()).with_status_code(204)
        }
        Err(e) => text(400, &format!("invalid callback payload: {e}")),
    }
}

/// Streams callbacks to one browser for as long as it keeps the connection open.
///
/// The response is written straight to the socket rather than through [`Response`]:
/// tiny_http's chunked encoder buffers 8 KiB before it writes anything, which would stall an
/// event stream indefinitely. Ending the body at connection close is the other half of that
/// trade, so the response says `Connection: close`.
fn handle_events<R: Runtime>(state: &State<R>, request: Request, cors: Vec<Header>) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let id = state.register_client(tx);

    let mut head = String::from("HTTP/1.1 200 OK\r\n");
    for header in &cors {
        head.push_str(&format!("{}: {}\r\n", header.field, header.value));
    }
    head.push_str("Content-Type: text/event-stream\r\n");
    head.push_str("Cache-Control: no-store\r\n");
    // Nothing should sit between us and the browser, but anything that does must not buffer.
    head.push_str("X-Accel-Buffering: no\r\n");
    head.push_str("Connection: close\r\n\r\n");
    head.push_str(&format!(
        "retry: 1000\nevent: ready\ndata: {}\n\n",
        serde_json::json!({ "clientId": id })
    ));

    let mut writer = request.into_writer();
    let mut write = |bytes: &[u8]| {
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .is_ok()
    };

    let mut connected = write(head.as_bytes());
    while connected {
        connected = match rx.recv_timeout(SSE_PING) {
            Ok(frame) => write(&frame),
            // A comment doubles as a keep-alive and as the write that notices a gone peer.
            Err(RecvTimeoutError::Timeout) => write(b": ping\n\n"),
            // Our sender was dropped: the client is already unregistered.
            Err(RecvTimeoutError::Disconnected) => break,
        };
    }

    state.unregister_client(id);
}

// ---------------------------------------------------------------------------------------
// GET /asset/<protocol>/<path>
// ---------------------------------------------------------------------------------------

fn handle_asset<R: Runtime>(state: &State<R>, request: Request, rest: &str, cors: Vec<Header>) {
    match open_asset(state, &request, rest) {
        Ok(asset) => {
            let mut headers = cors;
            headers.push(header("Content-Type", asset.content_type));
            // Media elements seek, which they can only do over a range-capable response.
            headers.push(header("Accept-Ranges", "bytes"));
            if let Some(range) = asset.content_range {
                headers.push(header("Content-Range", &range));
            }

            let response = Response::new(
                StatusCode(asset.status),
                headers,
                asset.reader,
                Some(asset.length as usize),
                None,
            );
            let _ = request.respond(response);
        }
        Err((status, message)) => respond(request, text(status, &message), cors),
    }
}

struct Asset {
    status: u16,
    reader: io::Take<File>,
    length: u64,
    content_type: &'static str,
    content_range: Option<String>,
}

fn open_asset<R: Runtime>(
    state: &State<R>,
    request: &Request,
    rest: &str,
) -> Result<Asset, (u16, String)> {
    if !state.config.serve_assets {
        return Err((403, "asset serving is disabled".into()));
    }

    let (protocol, encoded) = rest
        .split_once('/')
        .ok_or((400, "expected /asset/<protocol>/<path>".to_string()))?;

    if protocol != "asset" {
        return Err((
            404,
            format!("the `{protocol}` protocol is not proxied by the dev server"),
        ));
    }

    let path = PathBuf::from(percent_decode(encoded));
    let describe = |e: std::io::Error| format!("{}: {e}", path.display());

    let mut file = File::open(&path).map_err(|e| (404, describe(e)))?;
    let len = file.metadata().map_err(|e| (500, describe(e)))?.len();

    let range = header_value(request, "range").and_then(|value| parse_range(&value, len));
    let (status, start, length, content_range) = match range {
        Some((start, end)) => (
            206,
            start,
            end - start + 1,
            Some(format!("bytes {start}-{end}/{len}")),
        ),
        None => (200, 0, len, None),
    };

    if start > 0 {
        file.seek(SeekFrom::Start(start))
            .map_err(|e| (500, describe(e)))?;
    }

    Ok(Asset {
        status,
        reader: file.take(length),
        length,
        content_type: content_type_of(&path),
        content_range,
    })
}

/// Parses a single-range `Range: bytes=start-end` header into inclusive bounds.
fn parse_range(value: &str, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let spec = value.trim().strip_prefix("bytes=")?;
    // Multi-range responses need multipart bodies; serving the whole file is a valid answer.
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = match (start.trim(), end.trim()) {
        ("", "") => return None,
        // `bytes=-500`: the last 500 bytes.
        ("", suffix) => {
            let suffix: u64 = suffix.parse().ok()?;
            (len.saturating_sub(suffix), len - 1)
        }
        (start, "") => (start.parse().ok()?, len - 1),
        (start, end) => (start.parse().ok()?, end.parse::<u64>().ok()?.min(len - 1)),
    };

    (start <= end && start < len).then_some((start, end))
}

fn content_type_of(path: &std::path::Path) -> &'static str {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match ext.as_str() {
        "aac" => "audio/aac",
        "avif" => "image/avif",
        "css" => "text/css",
        "flac" => "audio/flac",
        "gif" => "image/gif",
        "htm" | "html" => "text/html",
        "ico" => "image/x-icon",
        "jpeg" | "jpg" => "image/jpeg",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "mp4" | "m4v" => "video/mp4",
        "ogg" => "audio/ogg",
        "ogv" => "video/ogg",
        "opus" => "audio/opus",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "txt" | "log" | "md" => "text/plain; charset=utf-8",
        "wasm" => "application/wasm",
        "wav" => "audio/wav",
        "webm" => "video/webm",
        "webp" => "image/webp",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Minimal `%XX` decoder. Command names and file paths are the only things that reach it.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(byte) = hex_pair(bytes[i + 1], bytes[i + 2]) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_pair(high: u8, low: u8) -> Option<u8> {
    Some((hex_digit(high)? << 4) | hex_digit(low)?)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DEFAULT_PORT;

    /// A stand-in for `std::env::var` so the tests never touch the process environment.
    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    fn environment_overrides_the_builder() {
        let mut config = Config {
            port: 3030,
            ..Default::default()
        };
        apply_env_overrides(
            &mut config,
            env(&[(HOST_ENV, "0.0.0.0"), (PORT_ENV, "3031")]),
        );

        assert_eq!(config.host, IpAddr::from([0, 0, 0, 0]));
        assert_eq!(config.port, 3031);
    }

    #[test]
    fn unset_variables_leave_the_builder_alone() {
        let mut config = Config {
            port: 4000,
            host: IpAddr::from([10, 0, 0, 1]),
            ..Default::default()
        };
        apply_env_overrides(&mut config, env(&[]));

        assert_eq!(config.port, 4000);
        assert_eq!(config.host, IpAddr::from([10, 0, 0, 1]));
    }

    #[test]
    fn unparseable_values_are_ignored() {
        let mut config = Config::default();
        apply_env_overrides(
            &mut config,
            env(&[
                (HOST_ENV, "not-an-address"),
                // A port the OS picks cannot work: the bridge script needs the address up front.
                (PORT_ENV, "0"),
                (ORIGINS_ENV, " , "),
            ]),
        );

        assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.origins, OriginPolicy::Auto);
    }

    #[test]
    fn accepts_localhost_as_a_host() {
        let mut config = Config::default();
        apply_env_overrides(&mut config, env(&[(HOST_ENV, "localhost")]));

        assert_eq!(config.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn parses_origin_lists() {
        let mut config = Config::default();
        apply_env_overrides(
            &mut config,
            env(&[(ORIGINS_ENV, " http://a:1420 , http://b:1420 ")]),
        );
        assert_eq!(
            config.origins,
            OriginPolicy::List(vec!["http://a:1420".into(), "http://b:1420".into()])
        );

        let mut config = Config::default();
        apply_env_overrides(&mut config, env(&[(ORIGINS_ENV, "*")]));
        assert_eq!(config.origins, OriginPolicy::Any);
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("plugin%3Afs%7Cread"), "plugin:fs|read");
        assert_eq!(percent_decode("/tmp/a%20b.png"), "/tmp/a b.png");
        // A stray `%` is left alone rather than swallowing the next characters.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        // Clamped to the file, and open-ended requests past the end are refused.
        assert_eq!(parse_range("bytes=0-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=2000-", 1000), None);
        assert_eq!(parse_range("bytes=0-99, 200-299", 1000), None);
        assert_eq!(parse_range("items=0-99", 1000), None);
        assert_eq!(parse_range("bytes=0-99", 0), None);
    }

    #[test]
    fn builds_origins_from_urls() {
        assert_eq!(
            origin_of(&Url::parse("http://localhost:1420/index.html").unwrap()),
            "http://localhost:1420"
        );
        assert_eq!(
            origin_of(&Url::parse("https://example.com").unwrap()),
            "https://example.com"
        );
    }

    #[test]
    fn base_url_rewrites_wildcard_hosts() {
        let mut config = Config::default();
        assert_eq!(base_url(&config), "http://127.0.0.1:3030");

        config.host = "0.0.0.0".parse().unwrap();
        config.port = 4000;
        assert_eq!(base_url(&config), "http://127.0.0.1:4000");
    }
}
