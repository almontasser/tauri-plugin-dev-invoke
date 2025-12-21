use super::DevInvokeState;
use std::sync::Arc;
use tauri::{AppHandle, Runtime};
use tiny_http::{Header, Method, Response, Server};

pub fn start<R: Runtime>(_app_handle: AppHandle<R>, state: Arc<DevInvokeState>, port: u16) {
    let addr = format!("127.0.0.1:{}", port);
    let server = Server::http(&addr).expect("Failed to start dev-invoke HTTP server");

    println!("[dev-invoke] HTTP server listening on http://{}", addr);

    for mut request in server.incoming_requests() {
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

        let mut content = String::new();
        if let Err(e) = request.as_reader().read_to_string(&mut content) {
            let _ = request.respond(
                Response::from_string(format!("Failed to read body: {}", e)).with_status_code(400),
            );
            continue;
        }

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

        let result = match state.handlers.get(&payload.cmd) {
            Some(handler) => handler(payload.args),
            None => Err(format!("Unknown command: {}", payload.cmd)),
        };

        let (status, body) = match result {
            Ok(val) => (200, serde_json::to_string(&val).unwrap()),
            Err(e) => (
                500,
                serde_json::to_string(&serde_json::json!({ "error": e })).unwrap(),
            ),
        };

        let response = Response::from_string(body)
            .with_status_code(status)
            .with_header(cors_header("Content-Type", "application/json"))
            .with_header(cors_header("Access-Control-Allow-Origin", "*"));

        let _ = request.respond(response);
    }
}

fn cors_header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}
