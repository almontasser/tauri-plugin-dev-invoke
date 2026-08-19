// Copyright (c) tauri-plugin-dev-invoke contributors
// Injected into every Tauri webview by `tauri-plugin-dev-invoke` while the dev server runs.
//
// Rust hands values back to JavaScript by evaluating
// `window.__TAURI_INTERNALS__.runCallback(id, payload)` inside a webview. Every `Channel`
// message, every event listener callback and every other Rust -> JS push goes through it.
// A browser tab is not a webview, so none of that ever reaches it on its own.
//
// This script makes the host webview relay any callback id it does not own back to the dev
// server, which fans it out to the connected browsers. `runCallback` resolves handlers with
// `internals.callbacks.get(id)`, so shadowing `get` on that Map intercepts unknown ids
// without having to touch `runCallback` itself (it is defined non-writable).
;(function () {
  var ENDPOINT = '__TEMPLATE_endpoint__'
  var BRIDGE_EVENT = '__TEMPLATE_bridge_event__'

  if (window.__TAURI_DEV_INVOKE_BRIDGE__) {
    return
  }

  var internals = window.__TAURI_INTERNALS__
  var callbacks = internals && internals.callbacks

  if (!callbacks || typeof callbacks.get !== 'function') {
    console.warn(
      '[dev-invoke] the Tauri callback registry is unavailable; browser clients will not ' +
        'receive events or channel messages'
    )
    return
  }

  var lookup = Map.prototype.get.bind(callbacks)

  // Binary values can sit anywhere in a callback payload, not just at the root: the channel
  // protocol wraps them as `{ message: ArrayBuffer, index }`. They are swapped for a marker
  // object that the browser turns back into an ArrayBuffer.
  var BINARY_KEY = '__devInvokeBinary__'
  var MAX_DEPTH = 32

  function toBase64(bytes) {
    var binary = ''
    var chunk = 0x8000
    for (var i = 0; i < bytes.length; i += chunk) {
      binary += String.fromCharCode.apply(
        null,
        bytes.subarray(i, Math.min(i + chunk, bytes.length))
      )
    }
    return btoa(binary)
  }

  function marker(bytes) {
    var wrapper = {}
    wrapper[BINARY_KEY] = toBase64(bytes)
    return wrapper
  }

  function encode(value, depth) {
    if (value === undefined) return null
    if (value === null || typeof value !== 'object') return value
    if (value instanceof ArrayBuffer) return marker(new Uint8Array(value))
    if (ArrayBuffer.isView(value)) {
      return marker(new Uint8Array(value.buffer, value.byteOffset, value.byteLength))
    }
    if (depth >= MAX_DEPTH) return value
    if (Array.isArray(value)) {
      return value.map(function (item) {
        return encode(item, depth + 1)
      })
    }

    var out = {}
    for (var key in value) {
      if (Object.prototype.hasOwnProperty.call(value, key)) {
        out[key] = encode(value[key], depth + 1)
      }
    }
    return out
  }

  // Messages are queued and flushed one batch at a time so browsers observe them in the
  // same order the webview did.
  var queue = []
  var flushing = false
  // `fetch` is the cheap path; a restrictive CSP can block it, in which case we fall back
  // to the event system, which always reaches Rust.
  var transport = 'fetch'

  function enqueue(message) {
    queue.push(message)
    if (!flushing) {
      flushing = true
      void flush()
    }
  }

  async function flush() {
    try {
      while (queue.length) {
        await send(queue.splice(0, queue.length))
      }
    } finally {
      flushing = false
    }
  }

  async function send(batch) {
    if (transport === 'fetch') {
      try {
        await fetch(ENDPOINT, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(batch)
        })
        return
      } catch (e) {
        transport = 'event'
        console.warn(
          '[dev-invoke] could not reach the dev server over HTTP, relaying callbacks ' +
            'through the event system instead',
          e
        )
      }
    }

    try {
      await internals.invoke('plugin:event|emit', {
        event: BRIDGE_EVENT,
        payload: batch
      })
    } catch (e) {
      console.error('[dev-invoke] failed to relay callbacks to the dev server', e)
    }
  }

  callbacks.get = function (id) {
    var local = lookup(id)
    if (local) {
      return local
    }
    // Not ours: assume a browser client owns it. Clients ignore ids they did not mint.
    return function (data) {
      enqueue({ id: id, value: encode(data, 0) })
    }
  }

  Object.defineProperty(window, '__TAURI_DEV_INVOKE_BRIDGE__', {
    value: Object.freeze({ version: 1, endpoint: ENDPOINT })
  })
})()
