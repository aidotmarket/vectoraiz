/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** GitHub Personal Access Token for release API authentication */
  readonly VITE_GITHUB_TOKEN?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
