use std::sync::Arc;
use tauri::{ipc::InvokeBody, AppHandle, Manager, Runtime};
use tiny_http::{Header, Method, Response, Server};

pub fn start<R: Runtime>(app_handle: AppHandle<R>, invoke_key: String, port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(&addr).expect("Failed to start dev-invoke HTTP server");

    println!("[dev-invoke] HTTP server listening on http://{}", addr);

    let invoke_key = Arc::new(invoke_key);

    for mut request in server.incoming_requests() {
        // Handle CORS preflight
        if request.method() == &Method::Options {
            let response = Response::empty(200)
                .with_header(cors_header("Access-Control-Allow-Origin", "*"))
                .with_header(cors_header("Access-Control-Allow-Methods", "POST, OPTIONS"))
                .with_header(cors_header("Access-Control-Allow-Headers", "Content-Type"));
            let _ = request.respond(response);
            continue;
        }

        if request.method() != &Method::Post {
            let _ =
                request.respond(Response::from_string("Method not allowed").with_status_code(405));
            continue;
        }

        // Read request body
        let mut content = String::new();
        if let Err(e) = request.as_reader().read_to_string(&mut content) {
            let _ = request.respond(
                Response::from_string(format!("Failed to read body: {}", e)).with_status_code(400),
            );
            continue;
        }

        // Parse the invoke payload
        #[derive(serde::Deserialize)]
        struct InvokePayload {
            cmd: String,
            #[serde(default)]
            args: serde_json::Value,
        }

        let payload: InvokePayload = match serde_json::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
                let _ = request.respond(
                    Response::from_string(format!("Invalid JSON: {}", e)).with_status_code(400),
                );
                continue;
            }
        };

        // Get the first available webview to route the invoke through
        let webviews = app_handle.webview_windows();
        let webview = match webviews.values().next() {
            Some(w) => w.clone(),
            None => {
                let _ = request.respond(
                    Response::from_string(r#"{"error":"No webview available yet"}"#)
                        .with_status_code(503)
                        .with_header(cors_header("Content-Type", "application/json"))
                        .with_header(cors_header("Access-Control-Allow-Origin", "*")),
                );
                continue;
            }
        };

        // Create an InvokeRequest to route through Tauri's real invoke system
        let invoke_request = tauri::webview::InvokeRequest {
            cmd: payload.cmd.clone(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: InvokeBody::Json(payload.args),
            headers: Default::default(),
            invoke_key: invoke_key.as_ref().clone(),
        };

        // Use a channel to capture the response
        let (tx, rx) = std::sync::mpsc::channel();
        let cmd = payload.cmd.clone();

        // Call the webview's invoke mechanism
        webview.as_ref().clone().on_message(
            invoke_request,
            Box::new(move |_webview, _cmd, response, _callback, _error| {
                let result = match response {
                    tauri::ipc::InvokeResponse::Ok(body) => match body {
                        tauri::ipc::InvokeResponseBody::Json(json) => Ok(json),
                        tauri::ipc::InvokeResponseBody::Raw(bytes) => {
                            Ok(serde_json::to_string(&bytes).unwrap_or_default())
                        }
                    },
                    tauri::ipc::InvokeResponse::Err(err) => Err(format!("{:?}", err)),
                };
                let _ = tx.send(result);
            }),
        );

        // Wait for response with timeout
        let result = match rx.recv_timeout(std::time::Duration::from_secs(30)) {
            Ok(Ok(json)) => (200, json),
            Ok(Err(e)) => (500, serde_json::json!({ "error": e }).to_string()),
            Err(_) => (
                500,
                serde_json::json!({ "error": format!("Timeout waiting for command: {}", cmd) })
                    .to_string(),
            ),
        };

        let response = Response::from_string(result.1)
            .with_status_code(result.0)
            .with_header(cors_header("Content-Type", "application/json"))
            .with_header(cors_header("Access-Control-Allow-Origin", "*"));

        let _ = request.respond(response);
    }
}

fn cors_header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}
