/**
 * Browser-side half of `tauri-plugin-dev-invoke`.
 *
 * Installs a stand-in for `window.__TAURI_INTERNALS__` that speaks HTTP to the plugin's dev
 * server instead of the webview IPC, so `@tauri-apps/api` and every Tauri plugin behave the
 * same way in a browser tab as they do inside the app.
 *
 * @module
 */

/** Metadata the dev server reports about the running app. */
export interface DevInvokeMetadata {
    /** Label of the window browser clients impersonate. */
    window: string;
    /** Label of the webview browser clients impersonate. */
    webview: string;
    /** Every webview label currently open. */
    webviews: string[];
    /** `macos`, `windows`, `linux`, ... */
    os: string;
    path: { sep: string; delimiter: string };
    assetBase: string;
    serveAssets: boolean;
    pluginVersion: string;
}

export interface DevInvokeOptions {
    /** Base URL of the dev server. Defaults to `http://localhost:3030`. */
    url?: string;
    /**
     * Window label to impersonate. Defaults to whatever the server reports, which is the
     * `main` window unless the plugin was configured otherwise.
     */
    window?: string;
    /**
     * Patch even when running inside a Tauri webview. Off by default, so calling this in
     * production code is harmless.
     */
    force?: boolean;
    /** Log connection details to the console. Defaults to `true`. */
    debug?: boolean;
}

/** Handle to the installed shim. */
export interface DevInvokeHandle {
    /** Base URL of the dev server. */
    url: string;
    /** Metadata reported by the server, once `/metadata` has answered. */
    metadata: DevInvokeMetadata | null;
    /** Removes the shim and closes the event stream. */
    teardown: () => void;
}

interface TauriMetadata {
    currentWindow: { label: string };
    currentWebview: { windowLabel: string; label: string };
}

interface TauriInternals {
    invoke: <T>(cmd: string, payload?: unknown, options?: InvokeOptions) => Promise<T>;
    transformCallback: (callback?: (payload: any) => void, once?: boolean) => number;
    unregisterCallback: (id: number) => void;
    runCallback: (id: number, data: unknown) => void;
    callbacks: Map<number, (payload: any) => void>;
    convertFileSrc: (filePath: string, protocol?: string) => string;
    metadata: TauriMetadata;
    plugins: { path?: { sep: string; delimiter: string } };
}

interface InvokeOptions {
    headers?: Headers | Record<string, string>;
}

declare global {
    interface Window {
        isTauri?: boolean;
        __TAURI_INTERNALS__?: TauriInternals;
        __TAURI_EVENT_PLUGIN_INTERNALS__?: {
            unregisterListener: (event: string, eventId: number) => void;
        };
    }
}

/** Matches `SERIALIZE_TO_IPC_FN` in `@tauri-apps/api/core`. */
const SERIALIZE_TO_IPC_FN = "__TAURI_TO_IPC_KEY__";

/** Tells resolve from reject, exactly like Tauri's own IPC does. */
const RESPONSE_HEADER = "Tauri-Response";

/** Marks a base64 binary value inside a callback payload. Set by the plugin's bridge script. */
const BINARY_KEY = "__devInvokeBinary__";

const DEFAULT_URL = "http://localhost:3030";

let handle: DevInvokeHandle | null = null;

/**
 * Routes `invoke()` and everything built on top of it through the dev server.
 *
 * Inside a Tauri webview this is a no-op, so it is safe to leave in your entry point.
 *
 * The shim is installed synchronously; the returned promise resolves once the server's
 * metadata has been read and the event stream is open. Await it before rendering if your app
 * reads `getCurrentWindow().label` during startup, or pass {@link DevInvokeOptions.window}.
 *
 * @example
 * ```typescript
 * import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";
 *
 * await setupDevInvoke();
 * ```
 */
export function setupDevInvoke(options: DevInvokeOptions = {}): Promise<DevInvokeHandle | null> {
    const debug = options.debug ?? true;
    const log = (...args: unknown[]) => {
        if (debug) console.log("[dev-invoke]", ...args);
    };

    if (typeof window === "undefined") {
        return Promise.resolve(null);
    }

    if (window.__TAURI_INTERNALS__ && !options.force) {
        log("running inside a Tauri webview, nothing to patch");
        return Promise.resolve(null);
    }

    if (handle) {
        return Promise.resolve(handle);
    }

    const url = (options.url ?? DEFAULT_URL).replace(/\/+$/, "");
    log(`routing Tauri IPC through ${url}`);

    const callbacks = new Map<number, (payload: any) => void>();

    function transformCallback(callback?: (payload: any) => void, once = false): number {
        const id = crypto.getRandomValues(new Uint32Array(1))[0]!;
        callbacks.set(id, (payload) => {
            if (once) unregisterCallback(id);
            return callback?.(payload);
        });
        return id;
    }

    function unregisterCallback(id: number): void {
        callbacks.delete(id);
    }

    function runCallback(id: number, data: unknown): void {
        const callback = callbacks.get(id);
        if (callback) {
            callback(data);
        } else if (debug) {
            console.warn(
                `[dev-invoke] no callback ${id}; this is expected if the app reloaded while ` +
                    `Rust was mid-operation, or if another tab owns it`,
            );
        }
    }

    const metadata: TauriMetadata = {
        currentWindow: { label: options.window ?? "main" },
        currentWebview: {
            windowLabel: options.window ?? "main",
            label: options.window ?? "main",
        },
    };

    // Sensible guesses so the shim is usable before `/metadata` answers. They are replaced
    // with the real values as soon as it does.
    const guessedWindows = /win(dows|32|64)/i.test(navigator.userAgent);
    const plugins = {
        path: { sep: guessedWindows ? "\\" : "/", delimiter: guessedWindows ? ";" : ":" },
    };
    let assetBase = `${url}/asset`;

    async function invoke<T>(cmd: string, payload: unknown = {}, opts?: InvokeOptions): Promise<T> {
        const { contentType, data } = processIpcMessage(payload);

        const headers = new Headers(opts?.headers ?? {});
        headers.set("Content-Type", contentType);
        headers.set("X-Dev-Invoke-Window", metadata.currentWebview.label);

        let response: Response;
        try {
            response = await fetch(`${url}/invoke/${encodeURIComponent(cmd)}`, {
                method: "POST",
                body: data,
                headers,
            });
        } catch (e) {
            throw new Error(
                `[dev-invoke] could not reach the dev server at ${url}. Is the app running ` +
                    `with the plugin enabled? (${String(e)})`,
            );
        }

        const status = response.headers.get(RESPONSE_HEADER);
        if (status === null) {
            // Not a command result: the server itself rejected the request.
            throw new Error(`[dev-invoke] ${response.status}: ${await response.text()}`);
        }

        const body = await readBody(response);
        if (status === "error") {
            throw body;
        }
        return body as T;
    }

    function convertFileSrc(filePath: string, protocol = "asset"): string {
        return `${assetBase}/${protocol}/${encodeURIComponent(filePath)}`;
    }

    const internals: TauriInternals = {
        invoke,
        transformCallback,
        unregisterCallback,
        runCallback,
        callbacks,
        convertFileSrc,
        metadata,
        plugins,
    };

    window.isTauri = true;
    window.__TAURI_INTERNALS__ = internals;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
        // Tauri drops the handler in the webview that registered it; here the browser owns it.
        unregisterListener: (_event: string, eventId: number) => unregisterCallback(eventId),
    };

    const events = new EventSource(`${url}/events`);
    events.addEventListener("callback", (event) => {
        let message: { id: number; value: unknown };
        try {
            message = JSON.parse((event as MessageEvent<string>).data);
        } catch (e) {
            console.error("[dev-invoke] malformed callback frame", e);
            return;
        }
        // Callbacks are broadcast to every connected tab; ignore the ones we did not mint.
        if (!callbacks.has(message.id)) return;
        runCallback(message.id, reviveBinary(message.value));
    });

    handle = {
        url,
        metadata: null,
        teardown() {
            events.close();
            callbacks.clear();
            delete window.__TAURI_INTERNALS__;
            delete window.__TAURI_EVENT_PLUGIN_INTERNALS__;
            delete window.isTauri;
            handle = null;
        },
    };

    const ready = Promise.all([
        once(events, "open").catch(() => {
            console.warn(
                `[dev-invoke] event stream to ${url} is not open yet; events and channels ` +
                    `will start flowing once it reconnects`,
            );
        }),
        fetch(`${url}/metadata`)
            .then((response) => response.json() as Promise<DevInvokeMetadata>)
            .then((served) => {
                const label = options.window ?? served.webview;
                metadata.currentWindow.label = options.window ?? served.window;
                metadata.currentWebview.label = label;
                metadata.currentWebview.windowLabel = metadata.currentWindow.label;
                plugins.path = served.path;
                assetBase = served.assetBase;
                if (handle) handle.metadata = served;
                log(`connected to ${served.os} app, impersonating webview "${label}"`);
            })
            .catch((e) => {
                console.warn(
                    `[dev-invoke] could not read metadata from ${url}; falling back to ` +
                        `defaults (window "${metadata.currentWindow.label}")`,
                    e,
                );
            }),
    ]).then(() => handle);

    return ready;
}

/** Whether the browser shim is currently installed. */
export function isDevInvokeActive(): boolean {
    return handle !== null;
}

/** The active shim, or `null` when running inside a Tauri webview. */
export function getDevInvokeHandle(): DevInvokeHandle | null {
    return handle;
}

/**
 * Serializes a command payload the way Tauri's own IPC does, so `Channel`, `Image` and any
 * other type with a `__TAURI_TO_IPC_KEY__` method survives the trip, and binary payloads are
 * sent as bytes rather than JSON arrays.
 */
function processIpcMessage(message: unknown): { contentType: string; data: BodyInit } {
    if (message instanceof ArrayBuffer || ArrayBuffer.isView(message)) {
        return { contentType: "application/octet-stream", data: message as BodyInit };
    }
    if (Array.isArray(message)) {
        // Tauri treats a top-level array as a byte payload. `fetch` would stringify the array,
        // so hand it the bytes the Rust side actually expects.
        return { contentType: "application/octet-stream", data: new Uint8Array(message) };
    }

    const data = JSON.stringify(message, (_key, value) => {
        if (value instanceof Map) return Object.fromEntries(value.entries());
        if (value instanceof Uint8Array) return Array.from(value);
        if (value instanceof ArrayBuffer) return Array.from(new Uint8Array(value));
        if (typeof value === "object" && value !== null && SERIALIZE_TO_IPC_FN in value) {
            return (value as Record<string, () => unknown>)[SERIALIZE_TO_IPC_FN]!();
        }
        return value;
    });

    return { contentType: "application/json", data };
}

/** Decodes a response the same way the webview's IPC does. */
async function readBody(response: Response): Promise<unknown> {
    const contentType = (response.headers.get("content-type") ?? "").split(";")[0]!.trim();
    switch (contentType) {
        case "application/json": {
            const text = await response.text();
            return text.length ? JSON.parse(text) : null;
        }
        case "text/plain":
            return response.text();
        default:
            return response.arrayBuffer();
    }
}

/**
 * Turns the bridge's binary markers back into `ArrayBuffer`s, wherever they sit in the
 * payload. Channel messages arrive wrapped as `{ message, index }`, so the root is not the
 * only place a buffer can appear.
 */
function reviveBinary(value: unknown): unknown {
    if (value === null || typeof value !== "object") return value;
    if (Array.isArray(value)) return value.map(reviveBinary);

    const record = value as Record<string, unknown>;
    const encoded = record[BINARY_KEY];
    if (typeof encoded === "string" && Object.keys(record).length === 1) {
        return decodeBase64(encoded);
    }

    const revived: Record<string, unknown> = {};
    for (const [key, item] of Object.entries(record)) {
        revived[key] = reviveBinary(item);
    }
    return revived;
}

function decodeBase64(value: string): ArrayBuffer {
    const binary = atob(value);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
        bytes[i] = binary.charCodeAt(i);
    }
    return bytes.buffer;
}

function once(target: EventTarget, event: string): Promise<void> {
    return new Promise((resolve, reject) => {
        target.addEventListener(event, () => resolve(), { once: true });
        target.addEventListener("error", () => reject(new Error(`${event} failed`)), { once: true });
    });
}
