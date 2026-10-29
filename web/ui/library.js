// The Library page: every project as a card or a row, with search, filters from
// the menu, sorting, and the Add button.
import { html, useEffect, useMemo, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, setPref, loadRepos } from "./state.js";
import { Icon } from "./icons.js";
import { openAdd } from "./dialogs.js";
import { launch, checkAll } from "./actions.js";
import { bytes, ago, when, plural } from "../lib/format.js";

const SORTS = [
  ["updated", "Updated upstream"],
  ["captured", "Captured"],
  ["added", "Added"],
  ["name", "Name"],
  ["size", "Size"],
  ["stars", "Stars"],
];

function sorter(by) {
  const t = (s) => (s ? Date.parse(s) || 0 : 0);
  switch (by) {
    case "name": return (a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
    case "captured": return (a, b) => t(b.last_changed) - t(a.last_changed);
    case "added": return (a, b) => t(b.added) - t(a.added);
    case "size": return (a, b) => b.bytes - a.bytes;
    case "stars": return (a, b) => b.stars - a.stars;
    default: return (a, b) => t(b.pushed_at) - t(a.pushed_at);
  }
}

function matches(r, f) {
  if (!f) return true;
  switch (f.by) {
    case "lang": return r.language === f.value;
    case "tag": return r.tags.includes(f.value);
    case "host": return r.host === f.value;
    case "status": return r.status === f.value;
    default: return true;
  }
}

const statusBadge = (r) => {
  if (r.status === "working") return html`<span class="badge working">Downloading…</span>`;
  if (r.status === "queued") return html`<span class="badge queued">Waiting</span>`;
  if (r.status === "error") return html`<span class="badge error" title=${r.error}>Check failed</span>`;
  return null;
};

function LaunchButton({ r }) {
  if (!r.launchable) return null;
  return html`<button type="button" class="small" title=${`Launch ${r.latest_release?.tag || "the latest release"}`}
    onClick=${(e) => { e.preventDefault(); e.stopPropagation(); launch(r.id); }}>${Icon.play(13)} Launch</button>`;
}

function Card({ r }) {
  const open = () => { location.hash = `#/repo/${r.id}`; };
  return html`<article class="card" tabindex="0" data-id=${r.id} onClick=${open} onKeyDown=${(e) => e.key === "Enter" && open()}>
    <div class="card-top">
      <div class="card-title"><h3>${r.name}</h3><div class="card-sub">${r.host} / ${r.owner}</div></div>
      ${statusBadge(r)}
    </div>
    <p class="card-desc">${r.description || html`<span class="muted">No description</span>`}</p>
    ${r.match === "readme" && html`<div class="match">Found in the readme</div>`}
    <div class="facts">
      ${r.latest_release && html`<span title="Latest release captured">${Icon.tag(12)}<b>${r.latest_release.tag}</b></span>`}
      ${r.language && html`<span>${Icon.code(12)}${r.language}</span>`}
      ${r.stars > 0 && html`<span title="Stars">${Icon.star(12)}${r.stars.toLocaleString()}</span>`}
      <span title="On disk, every version">${bytes(r.bytes)}</span>
      <span title="Versions of the code kept">${plural(r.snapshots, "version")}</span>
    </div>
    <div class="card-foot">
      <span class="when" title=${`Updated upstream ${when(r.pushed_at)}\nLast checked ${when(r.last_checked) || "never"}`}>
        ${r.pushed_at ? `Updated ${ago(r.pushed_at)}` : "Not checked yet"}</span>
      <${LaunchButton} r=${r} />
    </div>
  </article>`;
}

function PendingCard({ job }) {
  return html`<article class="card pending" data-pending=${job.repo}>
    <div class="card-top"><div class="card-title"><h3>${job.repo.split("/").pop()}</h3><div class="card-sub">${job.repo.split("/").slice(0, -1).join(" / ")}</div></div>
      <span class="badge working">Adding…</span></div>
    <p class="card-desc muted">${job.stage || "Waiting its turn"}</p>
  </article>`;
}

function Rows({ list, sort }) {
  const head = (key, label, cls = "") => html`<th class=${cls}><button type="button" onClick=${() => setPref({ sort: key })}>${label}${sort === key ? " ▾" : ""}</button></th>`;
  return html`<table class="list">
    <thead><tr>${head("name", "Project")}<th>Latest release</th>${head("updated", "Updated upstream")}${head("captured", "Captured")}${head("size", "Size", "num")}<th></th></tr></thead>
    <tbody>${list.map((r) => html`<tr class="row" key=${r.id} data-id=${r.id} onClick=${() => { location.hash = `#/repo/${r.id}`; }}>
      <td><div class="name">${r.name} ${statusBadge(r)}</div><div class="sub">${r.host} / ${r.owner}</div></td>
      <td class="mono">${r.latest_release?.tag || html`<span class="muted">none</span>`}</td>
      <td class="date" title=${when(r.pushed_at)}>${ago(r.pushed_at)}</td>
      <td class="date" title=${when(r.last_changed)}>${ago(r.last_changed)}</td>
      <td class="num">${bytes(r.bytes)}</td>
      <td class="num"><${LaunchButton} r=${r} /></td>
    </tr>`)}</tbody>
  </table>`;
}

function EmptyLibrary() {
  const [text, setText] = useState("");
  return html`<div class="empty-state">
    <h2>Add your first project</h2>
    <p class="muted">Paste a link to a GitHub, GitLab or Codeberg project. Repo Sync downloads its code and its latest release, keeps them here, and checks for new ones on a schedule, keeping every version.</p>
    <form onSubmit=${(e) => { e.preventDefault(); openAdd(text); }}>
      <input id="first-link" type="url" placeholder="https://github.com/owner/name" value=${text} onInput=${(e) => setText(e.target.value)} aria-label="Link to a project" />
      <button type="submit" class="primary">${Icon.plus(15)} Add</button>
    </form>
  </div>`;
}

export function LibraryPage() {
  const repos = useStore(ui, (s) => s.repos);
  const totals = useStore(ui, (s) => s.totals);
  const filter = useStore(ui, (s) => s.filter);
  const view = useStore(ui, (s) => s.view);
  const sort = useStore(ui, (s) => s.sort);
  const jobs = useStore(ui, (s) => s.jobs);
  const [query, setQuery] = useState(ui.get().query);
  // the search runs in the backend (it reads the readmes too), a moment after typing stops
  useEffect(() => {
    if (query === ui.get().query) return;
    const t = setTimeout(() => { ui.set({ query }); loadRepos(); }, 200);
    return () => clearTimeout(t);
  }, [query]);
  const list = useMemo(() => repos.filter((r) => matches(r, filter)).sort(sorter(sort)), [repos, filter, sort]);
  const have = new Set(repos.map((r) => r.id));
  const pending = [jobs.current && { ...jobs.current.job, stage: jobs.current.stage }, ...(jobs.queued || [])]
    .filter((j) => j && j.type === "add" && !have.has(j.repo));
  const empty = totals.count === 0 && pending.length === 0;
  return html`<div class="cmdbar">
      <h1>Library</h1>
      <span class="count">${filter ? `${list.length} of ${totals.count}` : totals.count}</span>
      ${filter && html`<button type="button" class="chip" onClick=${() => ui.set({ filter: null })} title="Show everything">${filter.value} ${Icon.x(11)}</button>`}
      <label class="search"><span class="visually-hidden">Search</span>${Icon.search(15)}
        <input id="search" type="search" placeholder="Search names, topics, readmes…" value=${query} onInput=${(e) => setQuery(e.target.value)} /></label>
      <span class="spacer"></span>
      <select aria-label="Sort by" value=${sort} onChange=${(e) => setPref({ sort: e.target.value })}>
        ${SORTS.map(([k, l]) => html`<option value=${k}>${l}</option>`)}</select>
      <div class="seg" role="group" aria-label="View">
        <button type="button" aria-pressed=${view === "cards" ? "true" : "false"} title="Cards" onClick=${() => setPref({ view: "cards" })}>${Icon.grid(16)}</button>
        <button type="button" aria-pressed=${view === "list" ? "true" : "false"} title="List" onClick=${() => setPref({ view: "list" })}>${Icon.list(16)}</button>
      </div>
      <button type="button" id="check-all" onClick=${checkAll} disabled=${!totals.count} title="Check every project for new versions now">${Icon.refresh(15)} Check all</button>
      <button type="button" class="primary" id="add" onClick=${() => openAdd("")} title="Add projects by their links (N)">${Icon.plus(15)} Add</button>
    </div>
    <div class="page">
      ${totals.unreadable.length > 0 && html`<p class="notice">${plural(totals.unreadable.length, "folder")} in the library couldn't be read: ${totals.unreadable.map((u) => `${u.folder} (${u.error})`).join("; ")}</p>`}
      ${empty ? html`<${EmptyLibrary} />`
        : list.length === 0 && pending.length === 0 ? html`<p class="muted">Nothing matches${query ? ` “${query}”` : ""}${filter ? ` in ${filter.value}` : ""}.</p>`
        : view === "list" ? html`<${Rows} list=${list} sort=${sort} />`
        : html`<div class="cards">${pending.map((j) => html`<${PendingCard} key=${"p" + j.id} job=${j} />`)}${list.map((r) => html`<${Card} key=${r.id} r=${r} />`)}</div>`}
    </div>`;
}
