# Tauri Dev Invoke API

JavaScript/TypeScript API adapter for `tauri-plugin-dev-invoke`.

## Purpose

Enables the standard `@tauri-apps/api` `invoke` function to work transparently in external browsers during development by routing requests to the local plugin server.

## Installation

```bash
npm install tauri-plugin-dev-invoke-api
```

## Usage

Call `setupDevInvoke()` at the entry point of your application (e.g., `main.tsx` or `main.ts`).

```typescript
import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

// Transparently patches __TAURI_INTERNALS__ in external browsers.
// Has no effect in the Tauri WebView.
setupDevInvoke();
```

Then use Tauri as normal:

```typescript
import { invoke } from "@tauri-apps/api/core";

invoke("greet", { name: "World" }).then(console.log);
```
