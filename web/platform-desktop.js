// Desktop app side of web/platform.js: the app's commands (desktop/core/src/api.rs,
// called through Tauri's "api" command in desktop/src-tauri/src/main.rs).

const tauri = globalThis.__TAURI__;
const invoke = (cmd, args) => tauri.core.invoke(cmd, args);
/** One of the app's commands, answered as JSON. */
const api = (cmd, args = {}) => invoke("api", { cmd, args });

export async function createPlatform() {
  const info = await api("app_info").catch((e) => ({ library_error: String(e), recent_libraries: [] }));
  let prefs = await api("prefs_get").catch(() => ({}));
  let timer = null;
  const store = {
    prefs: {
      get(key, fallback = null) { return key in prefs ? prefs[key] : fallback; },
      set(key, value) {
        prefs = { ...prefs, [key]: value };
        clearTimeout(timer);
        timer = setTimeout(() => api("prefs_set", { prefs }).catch(() => {}), 250);
      },
    },
  };
  const libraryUrl = info.library_url || "library://localhost/";
  // web links (in readmes, release notes) open in the default browser, not in the app's window
  document.addEventListener("click", (e) => {
    const a = e.target.closest?.("a[href]");
    if (!a || e.defaultPrevented) return;
    let url;
    try { url = new URL(a.href, location.href); } catch { return; }
    if (!/^https?:$/.test(url.protocol) || url.origin === location.origin || url.href.startsWith(libraryUrl)) return;
    e.preventDefault();
    invoke("open_url", { url: url.href }).catch((err) => console.warn("open_url", err));
  });
  return {
    kind: "desktop",
    info,
    store,
    api,
    openUrl: (url) => invoke("open_url", { url }).catch((err) => console.warn("open_url", err)),
    openPath: (path) => invoke("open_path", { path }),
    pickFolder: (title) => invoke("pick_folder", { title }),
    async refreshInfo() { Object.assign(info, await api("app_info")); return info; },
  };
}
