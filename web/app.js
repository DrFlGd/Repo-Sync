// Repo Sync: start-up and routing. The interface itself is in ui/ (Preact + htm,
// no build step); the backend is reached through platform.js.
import { createPlatform } from "./platform.js";
import { mountShell } from "./ui/shell.js";
import { ui, initState, loadRepos, startPolling } from "./ui/state.js";

// errors the tests read back (tests/desktop_page.py)
window.__errors = [];
window.addEventListener("error", (e) => window.__errors.push(String(e.message)));
window.addEventListener("unhandledrejection", (e) => window.__errors.push(String(e.reason?.message || e.reason)));

/** "#/repo/github.com/o/n?tab=releases" -> { page: "repo", id, tab }. */
export function routeFromHash(hash) {
  const [path, query = ""] = (hash || "").replace(/^#\/?/, "").split("?");
  const q = new URLSearchParams(query);
  if (path.startsWith("repo/")) return { page: "repo", id: decodeURIComponent(path.slice(5)), tab: q.get("tab") || "readme" };
  if (["activity", "settings", "about"].includes(path)) return { page: path };
  return { page: "library" };
}

function route() {
  ui.set({ route: routeFromHash(location.hash), menu: null });
  document.querySelector(".repo-main, .page")?.scrollTo?.(0, 0);
}

async function start() {
  const platform = await createPlatform();
  initState(platform);
  mountShell();
  route();
  window.addEventListener("hashchange", route);
  await loadRepos();
  ui.set({ ready: true });
  startPolling();
}

start();
