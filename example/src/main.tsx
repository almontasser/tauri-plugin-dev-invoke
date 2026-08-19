import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

// Routes Tauri IPC through the dev server when the app runs in a browser tab.
// Inside the Tauri webview this is a no-op. Awaiting it means the window label and path
// separator are already correct on the first render.
//
// `url` is only needed when the server is not on its default port — pair it with
// `DEV_INVOKE_PORT` to run several instances of the app at once.
await setupDevInvoke({ url: import.meta.env.VITE_DEV_INVOKE_URL });

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
