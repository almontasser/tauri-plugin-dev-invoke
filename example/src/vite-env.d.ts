/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Base URL of the dev-invoke server. Unset means the plugin's default port. */
  readonly VITE_DEV_INVOKE_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
