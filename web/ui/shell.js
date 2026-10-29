// Mounts the interface: the menu on the left, the page for the current route,
// the status bar and dialogs. Also the keys and pasting a link anywhere.
import { html, render, useMemo } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, ctx, setPref } from "./state.js";
import { Icon, Mark } from "./icons.js";
import { LibraryPage } from "./library.js";
import { RepoPage } from "./repo.js";
import { ActivityPage } from "./activity.js";
import { SettingsPage } from "./settings.js";
import { AboutPage } from "./about.js";
import { Dialogs, openAdd } from "./dialogs.js";
import { bytes, ago } from "../lib/format.js";

function NavLink({ href, icon, label, count, active }) {
  return html`<li><a class=${"nav-link" + (active ? " active" : "")} href=${href} aria-current=${active ? "page" : null} title=${label}>
    ${icon}<span class="label">${label}</span><span class="nav-count">${count ?? ""}</span></a></li>`;
}

function FilterLink({ by, value, label, count }) {
  const f = useStore(ui, (s) => s.filter);
  const page = useStore(ui, (s) => s.route.page);
  const active = page === "library" && f?.by === by && f?.value === value;
  const pick = () => { ui.set({ filter: active ? null : { by, value } }); if (page !== "library") location.hash = "#/"; };
  return html`<li><button type="button" class=${"nav-link" + (active ? " active" : "")} aria-pressed=${active ? "true" : "false"} onClick=${pick} title=${label}>
    <span class="label">${label}</span><span class="nav-count">${count}</span></button></li>`;
}

/** Counts of each value of `key` across the projects, most first. */
function tally(repos, get) {
  const m = new Map();
  for (const r of repos) for (const v of [].concat(get(r) || [])) if (v) m.set(v, (m.get(v) || 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

function Sidebar() {
  const route = useStore(ui, (s) => s.route);
  const repos = useStore(ui, (s) => s.repos);
  const count = useStore(ui, (s) => s.totals.count);
  const jobs = useStore(ui, (s) => s.jobs);
  const query = useStore(ui, (s) => s.query);
  const theme = useStore(ui, (s) => s.theme);
  const working = (jobs.current ? 1 : 0) + (jobs.queued?.length || 0);
  // filters count the whole library, not the search
  const groups = useMemo(() => query ? null : {
    hosts: tally(repos, (r) => r.host),
    langs: tally(repos, (r) => r.language),
    tags: tally(repos, (r) => r.tags),
  }, [repos, query]);
  const errors = repos.filter((r) => r.status === "error").length;
  const nextTheme = { system: "light", light: "dark", dark: "system" }[theme] || "system";
  const themeIcon = theme === "system" ? Icon.auto(16) : theme === "light" ? Icon.sun(16) : Icon.moon(16);
  return html`<nav class="sidebar" aria-label="Menu">
    <a class="brand" href="#/"><${Mark} /><span>Repo Sync</span></a>
    <ul class="nav">
      <${NavLink} href="#/" icon=${Icon.library(16)} label="Library" count=${count || ""} active=${route.page === "library" || route.page === "repo"} />
      <${NavLink} href="#/activity" icon=${Icon.activity(16)} label="Activity" count=${working || ""} active=${route.page === "activity"} />
      <${NavLink} href="#/settings" icon=${Icon.cog(16)} label="Settings" active=${route.page === "settings"} />
      <${NavLink} href="#/about" icon=${Icon.info(16)} label="About" active=${route.page === "about"} />
    </ul>
    ${groups && html`<div class="filters">
      ${errors > 0 && html`<div class="nav-head">Needs a look</div><ul class="nav"><${FilterLink} by="status" value="error" label="Check failed" count=${errors} /></ul>`}
      ${groups.tags.length > 0 && html`<div class="nav-head">Tags</div><ul class="nav">${groups.tags.map(([v, n]) => html`<${FilterLink} key=${v} by="tag" value=${v} label=${v} count=${n} />`)}</ul>`}
      ${groups.langs.length > 1 && html`<div class="nav-head">Languages</div><ul class="nav">${groups.langs.slice(0, 12).map(([v, n]) => html`<${FilterLink} key=${v} by="lang" value=${v} label=${v} count=${n} />`)}</ul>`}
      ${groups.hosts.length > 1 && html`<div class="nav-head">Sources</div><ul class="nav">${groups.hosts.map(([v, n]) => html`<${FilterLink} key=${v} by="host" value=${v} label=${v} count=${n} />`)}</ul>`}
    </div>`}
    <ul class="nav nav-foot">
      <li><button type="button" class="nav-link" id="theme-toggle" title=${`Theme: ${theme}. Click for ${nextTheme}.`} onClick=${() => setPref({ theme: nextTheme })}>
        ${themeIcon}<span class="label">Theme: ${theme[0].toUpperCase() + theme.slice(1)}</span></button></li>
    </ul>
  </nav>`;
}

function Main() {
  const route = useStore(ui, (s) => s.route);
  const info = useStore(ui, (s) => s.info);
  if (!info.library && route.page !== "settings" && route.page !== "about") {
    return html`<div class="page"><div class="empty-state">
      <h2>No library is open</h2>
      <p class="notice error">${info.library_error || "The library folder couldn't be opened."}</p>
      <div><a class="button primary" href="#/settings">Choose a library folder…</a></div>
    </div></div>`;
  }
  if (route.page === "repo") return html`<${RepoPage} key=${route.id} id=${route.id} tab=${route.tab} />`;
  if (route.page === "activity") return html`<${ActivityPage} />`;
  if (route.page === "settings") return html`<${SettingsPage} />`;
  if (route.page === "about") return html`<${AboutPage} />`;
  return html`<${LibraryPage} />`;
}

function StatusBar() {
  const s = useStore(ui, (st) => ({ info: st.info, totals: st.totals, jobs: st.jobs, ready: st.ready }));
  if (!s.ready) return null;
  const cur = s.jobs.current;
  const queued = s.jobs.queued?.length || 0;
  const pct = cur && cur.total > 0 ? Math.min(100, (cur.done / cur.total) * 100) : null;
  const lib = s.info.library;
  return html`<div class="statusbar-inner">
    <span class="lib" title=${lib?.path || ""}>${lib ? lib.path : "No library"}</span>
    <span>${s.totals.count} ${s.totals.count === 1 ? "project" : "projects"}</span>
    <span>${bytes(s.totals.bytes || 0)}</span>
    ${s.totals.next_check && html`<span title="The next scheduled check">Next check ${ago(s.totals.next_check)}</span>`}
    <span class="job">${cur
      ? html`<div class=${"meter" + (pct === null ? " busy" : "")}><i style=${pct === null ? "" : `width:${pct}%`}></i></div>
          <a href="#/activity">${cur.job.repo.split("/").slice(1).join("/")}: ${cur.stage}${pct !== null ? ` (${bytes(cur.done)} of ${bytes(cur.total)})` : cur.done ? ` (${bytes(cur.done)})` : ""}${queued ? ` · ${queued} waiting` : ""}</a>`
      : html`<span class="muted">Idle</span>`}</span>
  </div>`;
}

export function mountShell() {
  const mount = (id, C) => { const el = document.getElementById(id); if (el) render(html`<${C} />`, el); };
  mount("sidebar-root", Sidebar);
  mount("main-root", Main);
  mount("statusbar", StatusBar);
  mount("dialog-root", Dialogs);
  window.__reposync = { ui, ctx }; // for tests
  addEventListener("keydown", (e) => {
    const typing = e.target.closest?.("input, textarea, select, [contenteditable]");
    if (e.key === "Escape" && ui.get().dialog) { ui.get().dialog.resolve?.(false); ui.set({ dialog: null }); return; }
    if (ui.get().dialog || typing) return;
    if ((e.key === "/" || (e.key === "k" && (e.ctrlKey || e.metaKey)))) {
      const s = document.getElementById("search");
      if (s) { e.preventDefault(); s.focus(); s.select(); }
    } else if (e.key === "n" && !e.ctrlKey && !e.metaKey && !e.altKey && ui.get().info.library) {
      e.preventDefault();
      openAdd("");
    }
  });
  // a link pasted anywhere but a text box opens Add with it
  addEventListener("paste", (e) => {
    if (e.target.closest?.("input, textarea") || ui.get().dialog || !ui.get().info.library) return;
    const text = e.clipboardData?.getData("text") || "";
    if (/^\s*(https?:\/\/|git@)\S+/m.test(text)) { e.preventDefault(); openAdd(text.trim()); }
  });
}
