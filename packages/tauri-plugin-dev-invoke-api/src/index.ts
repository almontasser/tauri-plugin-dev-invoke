declare global {
    interface Window {
        __TAURI_INTERNALS__?: {
            invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
        };
    }
}

/**
 * Call this once at app startup to enable invoke() in external browsers.
 * In Tauri webview, this is a no-op.
 */
export function setupDevInvoke(httpEndpoint = "http://localhost:3030") {
    // If we're already in Tauri, do nothing
    if (window.__TAURI_INTERNALS__) {
        console.log("[dev-invoke] Running in Tauri webview, no patching needed.");
        return;
    }

    console.log("[dev-invoke] External browser detected, patching __TAURI_INTERNALS__");

    // Create a mock __TAURI_INTERNALS__ that routes to our HTTP server
    window.__TAURI_INTERNALS__ = {
        invoke: async <T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> => {
            console.log(`[dev-invoke] HTTP invoke: ${cmd}`);

            const response = await fetch(httpEndpoint, {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({ cmd, args }),
            });

            if (!response.ok) {
                const errorData = await response.json().catch(() => ({ error: "Unknown error" }));
                throw new Error(errorData.error || `HTTP ${response.status}`);
            }

            return response.json();
        },
    };
}
