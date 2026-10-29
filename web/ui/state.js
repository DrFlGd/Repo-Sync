// Shared state for the interface: the platform (the backend), the library's
// projects and jobs as last read, preferences, the theme, and a toast.
import { createStore } from "../lib/store.js";

export const ui = createStore({
  ready: false,
  route: { page: "library" },
  info: {},
  repos: [],           // summaries (api "repos")
  totals: { count: 0, bytes: 0, unreadable: [], next_check: null },
  query: "",
  filter: null,        // { by: "lang" | "tag" | "topic" | "host" | "status", value }
  jobs: { current: null, queued: [], recent: [] },
  dialog: null,        // { type: "add", text } | { type: "confirm", ... }
  view: "cards",
  sort: "updated",
  theme: "system",
});

let platform = null;
export const ctx = () => platform;
export const api = (cmd, args = {}) => platform.api(cmd, args);

export function initState(p) {
  platform = p;
  const prefs = p.store.prefs;
  ui.set({ info: p.info || {}, view: prefs.get("view", "cards"), sort: prefs.get("sort", "updated"), theme: prefs.get("theme", "system") });
  applyTheme();
  matchMedia("(prefers-color-scheme: dark)").addEventListener("change", applyTheme);
}

export function setPref(patch) {
  ui.set(patch);
  for (const [k, v] of Object.entries(patch)) platform.store.prefs.set(k, v);
  if ("theme" in patch) applyTheme();
}

function applyTheme() {
  const t = ui.get().theme;
  const dark = t === "dark" || (t === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.theme = dark ? "dark" : "light";
}

let toastTimer = null;
export function toast(text, ms = 3200) {
  const el = document.getElementById("toast");
  if (!el) return;
  el.textContent = text;
  el.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { el.hidden = true; }, ms);
}

/** An error from the backend in words. */
export const errText = (e) => (typeof e === "string" ? e : e?.message || String(e));

// ---- the library and jobs, read again while anything is happening ----------

let reposSeq = 0;
export async function loadRepos() {
  const seq = ++reposSeq;
  try {
    const r = await api("repos", { query: ui.get().query });
    if (seq !== reposSeq) return;
    ui.set({ repos: r.repos, totals: { count: r.count, bytes: r.bytes, unreadable: r.unreadable || [], next_check: r.next_check } });
  } catch (e) {
    ui.set({ repos: [], totals: { count: 0, bytes: 0, unreadable: [], next_check: null } });
  }
}

const busy = (j) => !!j.current || (j.queued || []).length > 0;
let lastFinished; // undefined until the first read: what finished before isn't announced
let pollTimer = null;

export async function loadJobs() {
  try {
    const j = await api("jobs");
    const prevBusy = busy(ui.get().jobs);
    ui.set({ jobs: j });
    const newest = j.recent?.[0];
    const key = newest ? `${newest.job.id}:${newest.finished}` : null;
    // something finished (or started): the library has changed
    if (key !== lastFinished || prevBusy !== busy(j) || busy(j)) {
      if (lastFinished !== undefined && key !== lastFinished && newest) announce(newest);
      lastFinished = key;
      await loadRepos();
    }
  } catch { /* no library */ }
}

function announce(f) {
  const name = f.job.repo.split("/").slice(1).join("/");
  if (!f.ok && !f.outcome) toast(`${name}: ${f.message}`, 6000);
  else if (f.outcome && (f.outcome.snapshot || f.outcome.release)) toast(`${name}: ${f.message}`);
}

/** Read jobs every second while there's work, every 15 s otherwise (the schedule may start some). */
export function startPolling() {
  const tick = async () => {
    await loadJobs();
    pollTimer = setTimeout(tick, busy(ui.get().jobs) ? 1000 : 15000);
  };
  clearTimeout(pollTimer);
  tick();
}

/** After starting work: look again soon. */
export function poke() {
  clearTimeout(pollTimer);
  setTimeout(startPolling, 250);
}

// "Release files to keep", named for this computer's system.
const SYSTEMS = { windows: "Windows", linux: "Linux", macos: "Mac" };
const thisSystem = () => SYSTEMS[ui.get().info.os] || "this computer";
export const fileChoices = () => [
  ["all", "Every file"],
  ["this-computer", `Only files for ${thisSystem()}`],
  ["none", "Only the release notes"],
];
export const filesHint = () => {
  const others = Object.values(SYSTEMS).filter((x) => x !== thisSystem()).join(" or ");
  return `Files named for ${others} (like -linux, -mac, .dmg, .deb), or for another processor (like arm64), are skipped. Files that name no system are kept.`;
};
