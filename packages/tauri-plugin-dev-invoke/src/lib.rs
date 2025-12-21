mod server;

use std::collections::HashMap;
use std::sync::Arc;
use tauri::{plugin::Builder, Runtime};

// Re-export the macro crate and paste
pub use paste;
pub use tauri_plugin_dev_invoke_macros::command;

pub type BoxHandler =
    Arc<dyn Fn(serde_json::Value) -> Result<serde_json::Value, String> + Send + Sync>;

pub struct DevInvokeState {
    pub handlers: HashMap<String, BoxHandler>,
}

pub fn init<R: Runtime>(state: DevInvokeState) -> tauri::plugin::TauriPlugin<R> {
    let state = Arc::new(state);

    Builder::new("dev-invoke")
        .setup(move |app, _| {
            #[cfg(debug_assertions)]
            {
                let app_handle = app.clone();
                let state_clone = state.clone();
                std::thread::spawn(move || {
                    server::start(app_handle, state_clone, 3030);
                });
            }
            let _ = app;
            let _ = state;
            Ok(())
        })
        .build()
}

#[macro_export]
macro_rules! dev_invoke_handler {
    ($($cmd:ident),* $(,)?) => {{
        let mut handlers: std::collections::HashMap<String, $crate::BoxHandler> =
            std::collections::HashMap::new();
        $(
            handlers.insert(
                stringify!($cmd).to_string(),
                std::sync::Arc::new(|args| {
                    $crate::paste::paste! { [<__dev_invoke_wrapper_ $cmd>](args) }
                })
            );
        )*
        $crate::DevInvokeState { handlers }
    }};
}
