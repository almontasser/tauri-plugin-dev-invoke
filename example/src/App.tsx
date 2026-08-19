import { useEffect, useRef, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { sep } from "@tauri-apps/api/path";
import "./App.css";

/** Keeps the newest entries first and the list short. */
function useLog(limit = 8) {
  const [entries, setEntries] = useState<string[]>([]);
  const push = (entry: string) =>
    setEntries((current) => [`${new Date().toLocaleTimeString()} — ${entry}`, ...current].slice(0, limit));
  return [entries, push] as const;
}

function App() {
  const [name, setName] = useState("");
  const [greeting, setGreeting] = useState("");
  const [label, setLabel] = useState("");
  const [progress, setProgress] = useState<number | null>(null);
  const [bytes, setBytes] = useState("");
  const [events, pushEvent] = useLog();
  const announcement = useRef<HTMLInputElement>(null);

  useEffect(() => {
    // `getCurrentWindow()` reads metadata the plugin reports; `whoami` proves the command
    // really ran against that webview.
    const current = getCurrentWindow().label;
    setLabel(current);
    invoke<string>("whoami")
      .then((routed) => setLabel(`${current} (routed through "${routed}")`))
      .catch(report);

    const unlisten = Promise.all([
      listen<number>("demo://tick", (event) => pushEvent(`tick #${event.payload}`)),
      listen<string>("demo://announcement", (event) => pushEvent(`announcement: ${event.payload}`)),
    ]).catch(report);

    return () => {
      void unlisten.then((fns) => fns && fns.forEach((fn) => fn()));
    };
  }, []);

  function report(error: unknown) {
    pushEvent(`error: ${error}`);
  }

  async function greet() {
    setGreeting(await invoke("greet", { name }));
  }

  async function countTo() {
    setProgress(0);
    const progressChannel = new Channel<number>((value) => setProgress(value));
    await invoke("count_to", { limit: 10, progress: progressChannel });
  }

  async function fetchBytes() {
    const data = await invoke<ArrayBuffer>("random_bytes", { count: 12 });
    setBytes(
      Array.from(new Uint8Array(data))
        .map((b) => b.toString(16).padStart(2, "0"))
        .join(" "),
    );
  }

  async function announce() {
    await invoke("broadcast", { message: announcement.current?.value || "hello" });
  }

  return (
    <main className="container">
      <h1>tauri-plugin-dev-invoke</h1>
      <p className="subtitle">
        Open this page in a browser and in the app window — both drive the same Rust backend.
      </p>

      <section>
        <h2>Window</h2>
        <p>
          <code>{label || "…"}</code> · path separator <code>{sep()}</code>
        </p>
      </section>

      <section>
        <h2>Command</h2>
        <form
          className="row"
          onSubmit={(e) => {
            e.preventDefault();
            void greet().catch(report);
          }}
        >
          <input onChange={(e) => setName(e.currentTarget.value)} placeholder="Enter a name..." />
          <button type="submit">Greet</button>
        </form>
        <p>{greeting}</p>
      </section>

      <section>
        <h2>Channel</h2>
        <button onClick={() => void countTo().catch(report)}>Count to 10</button>
        <p>{progress === null ? "not started" : `received ${progress}`}</p>
      </section>

      <section>
        <h2>Binary response</h2>
        <button onClick={() => void fetchBytes().catch(report)}>Fetch 12 bytes</button>
        <p>
          <code>{bytes || "—"}</code>
        </p>
      </section>

      <section>
        <h2>Events</h2>
        <div className="row">
          <input ref={announcement} placeholder="Announce something..." />
          <button onClick={() => void announce().catch(report)}>Emit</button>
        </div>
        <ul className="log">
          {events.map((entry, i) => (
            <li key={i}>{entry}</li>
          ))}
        </ul>
      </section>
    </main>
  );
}

export default App;
