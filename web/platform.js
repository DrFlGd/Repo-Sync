// Chooses how the page reaches the app's backend. Everything else in web/ talks
// only to the platform object.
//
// createPlatform() -> { kind, info, store, api(cmd, args), openUrl(url), openPath(path), pickFolder(title), refreshInfo() }
//   info:  app_info (version, os, the open library, recent libraries)
//   api:   one of the app's commands (desktop/core/src/api.rs), answered as JSON
//   store: prefs.get(key, fallback) / prefs.set(key, value), small UI preferences, synchronous

export async function createPlatform() {
  if (globalThis.__TAURI_INTERNALS__) {
    const desktop = await import("./platform-desktop.js");
    return desktop.createPlatform();
  }
  // opened as a plain web page: there's no backend
  const prefs = new Map();
  return {
    kind: "browser",
    info: { version: "", library: null, library_error: "Open Repo Sync in the desktop app to use a library.", recent_libraries: [] },
    store: { prefs: { get: (k, fallback = null) => (prefs.has(k) ? prefs.get(k) : fallback), set: (k, v) => prefs.set(k, v) } },
    api: async () => { throw new Error("No library here: open Repo Sync in the desktop app."); },
    openUrl: (url) => window.open(url, "_blank", "noopener"),
    openPath: async () => {},
    pickFolder: async () => null,
    async refreshInfo() { return this.info; },
  };
}
