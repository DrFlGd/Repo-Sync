// A project's page: its readme, every captured version of its code, every
// captured release (with Launch), and on the right its details, what it follows
// and its tags.
import { html, useEffect, useRef, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, api, toast, errText, loadRepos, fileChoices, filesHint } from "./state.js";
import { Icon } from "./icons.js";
import { confirmDialog } from "./dialogs.js";
import { launch, check, openFolder, openUrl } from "./actions.js";
import { bytes, ago, when, day, plural, short } from "../lib/format.js";

const KIND = { github: "GitHub", gitlab: "GitLab", gitea: "Gitea / Forgejo" };

/** A small menu under a button; closes on a click outside or Escape. */
function Menu({ label, title, cls = "", children, align = "left", id }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return;
    const off = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
    const esc = (e) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", off);
    document.addEventListener("keydown", esc);
    return () => { document.removeEventListener("mousedown", off); document.removeEventListener("keydown", esc); };
  }, [open]);
  return html`<span class="anchor" ref=${ref}>
    <button type="button" id=${id} class=${cls} title=${title} aria-haspopup="menu" aria-expanded=${open ? "true" : "false"} onClick=${() => setOpen(!open)}>${label}</button>
    ${open && html`<div class="menu" role="menu" style=${`top:calc(100% + 4px);${align === "right" ? "right:0" : "left:0"}`} onClick=${(e) => e.target.closest("button") && setOpen(false)}>${children}</div>`}
  </span>`;
}

function LaunchControl({ repo, onChange }) {
  const [list, setList] = useState(null);
  const s = repo.summary;
  useEffect(() => {
    if (!s.launchable) { setList(null); return; }
    api("launch_list", { id: s.id }).then(setList).catch(() => setList(null));
  }, [s.id, s.launchable, repo.launch?.file]);
  if (!s.launchable) {
    return html`<button type="button" class="primary" disabled title=${repo.releases.length ? "The captured releases have no files to launch" : "No release has been captured yet"}>${Icon.play(14)} Launch</button>`;
  }
  const chosen = list?.chosen || "";
  const name = chosen ? chosen.split("/").pop() : "";
  return html`<span class="split">
    <button type="button" class="primary" id="launch" title=${chosen ? `Start ${chosen} from ${list.folder}` : "Nothing to launch on this computer"} disabled=${list && !list.candidates.length}
      onClick=${() => launch(s.id)}>${Icon.play(14)} Launch ${s.launchable}</button>
    <${Menu} cls="primary" id="launch-menu" title="Choose what Launch starts" label=${Icon.down(14)}>
      <div class="menu-head">Launch starts (from ${list?.folder || s.launchable})</div>
      ${(list?.candidates || []).map((c) => html`<button type="button" role="menuitemradio" aria-checked=${c.file === chosen ? "true" : "false"}
        onClick=${async () => { await launch(s.id, { file: c.file, remember: true }); onChange(); }}>
        <span class="check-mark">${c.file === chosen ? Icon.check(13) : ""}</span>
        <span><span class="mono">${c.file}</span> <span class="muted">${c.kind === "archive" ? "(unpacked, then started)" : c.kind}</span></span></button>`)}
      ${list && !list.candidates.length && html`<div class="menu-head">Nothing in it runs on this computer</div>`}
      <div class="sep"></div>
      <button type="button" onClick=${() => openFolder(s.id, `releases/${list?.folder || s.launchable}`)}>${Icon.folder(14)} Open the release's folder</button>
    </${Menu}>
  </span>`;
}

function Readme({ repo, snapshot, setSnapshot }) {
  const [doc, setDoc] = useState(null);
  const [err, setErr] = useState("");
  const latest = repo.snapshots[repo.snapshots.length - 1];
  const folder = snapshot || latest?.folder;
  useEffect(() => {
    setErr("");
    if (!folder) { setDoc({ html: null }); return; }
    api("readme", { id: repo.summary.id, snapshot: folder }).then(setDoc).catch((e) => setErr(errText(e)));
  }, [repo.summary.id, folder]);
  if (!repo.snapshots.length) {
    return html`<p class="muted">${repo.follow.source ? "The code hasn't been captured yet, so there's no readme to show." : "This project doesn't follow its code, so there's no readme. Tick “The code” on the right to capture it."}</p>`;
  }
  const snap = repo.snapshots.find((s) => s.folder === folder) || latest;
  return html`<div>
    <div class="readme-src">
      <span>${doc?.file || "No readme"} · ${short(snap.commit)} · captured ${day(snap.captured)}</span>
      ${snap !== latest && html`<span class="badge warn">An earlier version</span><button type="button" class="small" onClick=${() => setSnapshot(null)}>Show the latest</button>`}
    </div>
    ${err ? html`<p class="notice error">${err}</p>`
      : !doc ? html`<p class="muted">Reading…</p>`
      : doc.html == null ? html`<p class="muted">This version has no readme.</p>`
      : html`<article class="readme" id="readme" dangerouslySetInnerHTML=${{ __html: doc.html }}></article>`}
  </div>`;
}

function Versions({ repo, showReadme, reload }) {
  const id = repo.summary.id;
  const list = [...repo.snapshots].reverse();
  if (!list.length) return html`<p class="muted">No version of the code has been captured yet.</p>`;
  const del = async (s) => {
    if (!(await confirmDialog({ title: "Delete this version?", body: `The files of ${short(s.commit)} (captured ${when(s.captured)}, ${bytes(s.bytes)}) are deleted from the library. Other versions stay.`, ok: "Delete version" }))) return;
    try { await api("version_delete", { id, kind: "source", folder: s.folder }); reload(); } catch (e) { toast(errText(e), 6000); }
  };
  return html`<table class="list" id="versions">
    <thead><tr><th>Captured</th><th>Commit</th><th>Committed</th><th class="num">Files</th><th class="num">Size</th><th></th></tr></thead>
    <tbody>${list.map((s, i) => html`<tr key=${s.folder}>
      <td class="date" title=${when(s.captured)}>${when(s.captured)} ${i === 0 && html`<span class="badge plain">latest</span>`}</td>
      <td><span class="mono">${short(s.commit)}</span> ${s.message}<div class="sub">${s.branch}</div></td>
      <td class="date" title=${when(s.commit_date)}>${day(s.commit_date)}</td>
      <td class="num">${s.files.toLocaleString()}</td>
      <td class="num">${bytes(s.bytes)}</td>
      <td class="num">
        ${s.readme && html`<button type="button" class="ghost small" title="Show this version's readme" onClick=${() => showReadme(s.folder)}>Readme</button>`}
        <button type="button" class="ghost small" title="Open this version's folder" onClick=${() => openFolder(id, `source/${s.folder}`)}>${Icon.folder(14)}</button>
        <button type="button" class="ghost small danger" title="Delete this version" onClick=${() => del(s)}>${Icon.trash(14)}</button>
      </td></tr>`)}</tbody>
  </table>`;
}

function Notes({ id, folder }) {
  const [doc, setDoc] = useState(null);
  useEffect(() => { api("release_notes", { id, folder }).then(setDoc).catch(() => setDoc({ html: "" })); }, [id, folder]);
  return html`<div class="notes"><div class="readme" dangerouslySetInnerHTML=${{ __html: doc?.html || "" }}></div></div>`;
}

// Every release the server lists, with a box to keep each one: ticking downloads
// it (beside the latest, which checks download by themselves), clearing a kept
// one deletes its files.
function Available({ repo, reload, del }) {
  const id = repo.summary.id;
  const [list, setList] = useState(null);
  const [err, setErr] = useState("");
  const [pre, setPre] = useState(true);
  const read = () => api("releases_available", { id }).then((l) => { setList(l); setErr(""); }).catch((e) => setErr(errText(e)));
  useEffect(() => { read(); }, [id, repo.releases.length, repo.summary.status]);
  const site = KIND[repo.source.kind] || repo.source.host;
  if (err) return html`<p class="notice error" id="available-error">Couldn't list the releases on ${site}: ${err}</p>`;
  if (!list) return html`<p class="muted">Asking ${site} for its releases…</p>`;
  if (!list.length) return html`<p class="muted">${site} lists no releases for this project.</p>`;
  const npre = list.filter((r) => r.prerelease).length;
  const shown = pre ? list : list.filter((r) => !r.prerelease || r.kept);
  const keep = async (r, on) => {
    if (on) {
      try { await api("release_get", { id, tag: r.tag }); toast(`Downloading ${r.tag}`); read(); loadRepos(); } catch (e) { toast(errText(e), 6000); }
    } else if (r.kept) {
      const k = repo.releases.find((x) => x.folder === r.kept);
      if (k && (await del(k))) read();
    }
  };
  return html`<section class="available" id="available">
    <h2 class="section-head">On ${site} <span class="n">${list.length}</span>
      <button type="button" class="ghost small" id="available-refresh" title=${`Ask ${site} for its releases again`} onClick=${read}>${Icon.refresh(13)}</button></h2>
    <p class="hint">Tick a release to download and keep it. Each check also downloads the newest release by itself (see Follow, on the right). Clearing a tick deletes that release's files.
      ${npre > 0 && html` <label class="check inline"><input type="checkbox" id="available-pre" checked=${pre} onChange=${(e) => setPre(e.target.checked)} /> Show ${npre} pre-release${npre === 1 ? "" : "s"}</label>`}</p>
    <table class="list">
      <thead><tr><th>Keep</th><th>Release</th><th>Published</th><th class="num">Files</th><th class="num">Size</th></tr></thead>
      <tbody>${shown.map((r) => html`<tr key=${r.tag} data-tag=${r.tag}>
        <td><input type="checkbox" aria-label=${`Keep ${r.tag}`} checked=${!!r.kept || r.waiting} disabled=${r.waiting} onChange=${(e) => keep(r, e.target.checked)} /></td>
        <td><span class="name">${r.name && r.name !== r.tag ? r.name : r.tag}</span>
          ${r.prerelease && html` <span class="badge warn">pre-release</span>`}
          ${r.kept && html` <span class="badge ok">kept</span>`}
          ${r.waiting && html` <span class="badge working">downloading…</span>`}
          ${r.name && r.name !== r.tag && html`<div class="sub">${r.tag}</div>`}</td>
        <td class="date" title=${when(r.published)}>${day(r.published)}</td>
        <td class="num">${r.files}</td>
        <td class="num">${r.bytes ? bytes(r.bytes) : ""}</td></tr>`)}</tbody>
    </table>
  </section>`;
}

function Releases({ repo, reload }) {
  const id = repo.summary.id;
  const [open, setOpen] = useState(() => new Set());
  const list = [...repo.releases].reverse();
  const del = async (r) => {
    if (!(await confirmDialog({ title: `Delete ${r.tag}?`, body: `The files of release ${r.tag} (${bytes(r.bytes)}) are deleted from the library. Other releases stay.`, ok: "Delete release" }))) return false;
    try { await api("version_delete", { id, kind: "release", folder: r.folder }); reload(); return true; } catch (e) { toast(errText(e), 6000); return false; }
  };
  const toggle = (f) => setOpen((s) => { const n = new Set(s); n.has(f) ? n.delete(f) : n.add(f); return n; });
  return html`<div>
  <${Available} repo=${repo} reload=${reload} del=${del} />
  <h2 class="section-head">Kept in the library <span class="n">${list.length}</span></h2>
  ${!list.length && html`<p class="muted">${repo.follow.releases ? "No release has been captured yet." : "This project doesn't follow its releases: tick “Releases” on the right, or a release above."}</p>`}
  <div class="versions" id="releases">${list.map((r, i) => html`<section class="release" key=${r.folder} data-release=${r.folder}>
    <div class="release-head">
      <h3>${r.name && r.name !== r.tag ? `${r.name}` : r.tag}</h3>
      ${r.name && r.name !== r.tag && html`<span class="mono muted">${r.tag}</span>`}
      ${i === 0 && html`<span class="badge plain">newest kept</span>`}
      ${r.prerelease && html`<span class="badge warn">pre-release</span>`}
      <span class="spacer" style="flex:1"></span>
      <button type="button" class="small" title="Start this release's program" disabled=${!r.assets.some((a) => a.file)} onClick=${() => launch(id, { folder: r.folder })}>${Icon.play(13)} Launch</button>
      <button type="button" class="ghost small" title="Open this release's folder" onClick=${() => openFolder(id, `releases/${r.folder}`)}>${Icon.folder(14)}</button>
      ${r.web_url && html`<button type="button" class="ghost small" title="Open the release's page" onClick=${() => openUrl(r.web_url)}>${Icon.external(14)}</button>`}
      <button type="button" class="ghost small danger" title="Delete this release" onClick=${() => del(r)}>${Icon.trash(14)}</button>
    </div>
    <div class="facts"><span>Published <b>${when(r.published) || "?"}</b></span><span>Captured <b>${when(r.captured)}</b></span><span>${bytes(r.bytes)}</span></div>
    ${r.assets.length > 0 ? html`<table class="list">
      <tbody>${r.assets.map((a) => html`<tr key=${a.name}>
        <td class="mono">${a.name}</td>
        <td>${a.file ? html`<span class="badge ok">kept</span>` : html`<span class="badge plain" title=${a.skipped}>not downloaded: ${a.skipped}</span>`}</td>
        <td class="num">${a.bytes ? bytes(a.bytes) : ""}</td></tr>`)}</tbody></table>`
      : html`<p class="hint">This release has no files of its own, only its notes.</p>`}
    <div><button type="button" class="ghost small" aria-expanded=${open.has(r.folder) ? "true" : "false"} onClick=${() => toggle(r.folder)}>${open.has(r.folder) ? "Hide the notes" : "Show the notes"}</button></div>
    ${open.has(r.folder) && html`<${Notes} id=${id} folder=${r.folder} />`}
  </section>`)}</div></div>`;
}

const CHECKS = [
  [null, "Library's schedule"],
  [1, "Every hour"],
  [6, "Every 6 hours"],
  [24, "Every day"],
  [168, "Every week"],
  [0, "Only by hand"],
];

// The project's branches, for the Branch list (null while asking, [] when the
// server can't say: then the branch is typed).
function useBranches(id) {
  const [list, setList] = useState(null);
  useEffect(() => { setList(null); api("branches", { id }).then(setList).catch(() => setList([])); }, [id]);
  return list;
}

function Following({ repo, save }) {
  const branches = useBranches(repo.summary.id);
  // kept here too, so a second change made before the first is saved builds on it
  const [f, setF] = useState(repo.follow);
  const latest = useRef(repo.follow);
  const key = JSON.stringify(repo.follow);
  useEffect(() => { latest.current = repo.follow; setF(repo.follow); }, [key]);
  const set = (patch) => {
    latest.current = { ...latest.current, ...patch };
    setF(latest.current);
    save(latest.current);
  };
  return html`<div class="follow" id="follow">
    <label class="check"><input type="checkbox" id="follow-source" checked=${f.source} onChange=${(e) => set({ source: e.target.checked })} /> The code</label>
    <div class="row" style="padding-left:24px"><span class="muted">Branch</span>
      ${branches && branches.length
        ? html`<select aria-label="Branch to follow" id="follow-branch" value=${f.branch} disabled=${!f.source} onChange=${(e) => set({ branch: e.target.value })}>
            <option value="">Default${repo.details.default_branch ? ` (${repo.details.default_branch})` : ""}</option>
            ${[...new Set([...(f.branch && !branches.includes(f.branch) ? [f.branch] : []), ...branches])].filter((b) => b !== repo.details.default_branch || f.branch === b).map((b) => html`<option value=${b}>${b}</option>`)}</select>`
        : html`<input type="text" placeholder=${branches ? repo.details.default_branch || "default" : "Reading the branches…"} value=${f.branch} disabled=${!f.source}
            onChange=${(e) => set({ branch: e.target.value.trim() })} aria-label="Branch to follow" />`}</div>
    <label class="check"><input type="checkbox" id="follow-releases" checked=${f.releases} onChange=${(e) => set({ releases: e.target.checked })} /> The newest release</label>
    <label class="check" style="padding-left:24px"><input type="checkbox" checked=${f.prereleases} disabled=${!f.releases} onChange=${(e) => set({ prereleases: e.target.checked })} /> including pre-releases</label>
    <div class="row" style="padding-left:24px">
      <select aria-label="Release files to keep" value=${f.files} disabled=${!f.releases} onChange=${(e) => set({ files: e.target.value })}>
        ${fileChoices().map(([k, l]) => html`<option value=${k}>${l}</option>`)}</select></div>
    ${f.releases && f.files === "this-computer" && html`<p class="hint" style="padding-left:24px">${filesHint()}</p>`}
    <div class="row"><span class="muted">Check</span>
      <select id="follow-every" aria-label="How often to check" value=${f.every_hours == null ? "" : String(f.every_hours)}
        onChange=${(e) => set({ every_hours: e.target.value === "" ? null : Number(e.target.value) })}>
        ${CHECKS.map(([v, l]) => html`<option value=${v == null ? "" : String(v)}>${l}</option>`)}</select></div>
  </div>`;
}

function Tags({ repo, saveTags }) {
  const [text, setText] = useState("");
  const add = (e) => {
    e.preventDefault();
    const t = text.trim();
    if (!t || repo.tags.includes(t)) { setText(""); return; }
    saveTags([...repo.tags, t]);
    setText("");
  };
  return html`<div>
    <ul class="chips">${repo.tags.map((t) => html`<li key=${t} class="chip tag">${t}<span class="x" role="button" tabindex="0" title=${`Remove ${t}`} onClick=${() => saveTags(repo.tags.filter((x) => x !== t))}>×</span></li>`)}
      </ul>
    <p class="hint">Your own labels for sorting the library (they're listed in the menu). They aren't the project's git tags or topics.</p>
    <form class="tag-edit" onSubmit=${add}><input id="tag-input" type="text" placeholder="Add a tag" value=${text} onInput=${(e) => setText(e.target.value)} />
      <button type="submit" class="small">Add</button></form>
  </div>`;
}

function Inspector({ repo, reload }) {
  const s = repo.summary;
  const d = repo.details;
  const id = s.id;
  // one save at a time, in the order made (requests sent together can land out of
  // order), and the page reloaded once the last one is in
  const queue = useRef({ last: Promise.resolve(), pending: 0 });
  const send = (cmd, args) => {
    const q = queue.current;
    q.pending++;
    q.last = q.last.then(() => api(cmd, args)).catch((e) => toast(errText(e), 6000)).finally(() => {
      if (--q.pending === 0) { reload(); loadRepos(); }
    });
  };
  const save = (follow) => send("follow_set", { id, follow });
  const saveTags = (tags) => send("tags_set", { id, tags });
  const latestSnap = repo.snapshots[repo.snapshots.length - 1];
  return html`<aside class="inspector" aria-label="Details">
    <section>
      <h2>Details</h2>
      <dl class="props" id="props">
        <dt>Source</dt><dd><a href=${repo.web_url} title=${repo.web_url}>${KIND[repo.source.kind] || repo.source.host}</a></dd>
        <dt>Size on disk</dt><dd id="prop-size">${bytes(s.bytes)}</dd>
        <dt>Kept</dt><dd>${plural(repo.snapshots.length, "version")}, ${plural(repo.releases.length, "release")}</dd>
        <dt>Added</dt><dd title=${when(repo.added)}>${day(repo.added)}</dd>
        <dt>Last captured</dt><dd title=${when(repo.last_changed)}>${repo.last_changed ? ago(repo.last_changed) : "never"}</dd>
        <dt>Last checked</dt><dd title=${when(repo.last_checked)}>${repo.last_checked ? ago(repo.last_checked) : "never"}</dd>
        <dt>Next check</dt><dd title=${when(s.due)}>${s.due ? ago(s.due) : "by hand"}</dd>
        <dt>Updated upstream</dt><dd title=${when(d.pushed_at)}>${d.pushed_at ? ago(d.pushed_at) : "?"}</dd>
        ${latestSnap && html`<dt>Latest commit</dt><dd title=${latestSnap.message}>${short(latestSnap.commit)} · ${day(latestSnap.commit_date)}</dd>`}
        ${repo.releases.length > 0 && html`<dt>Latest release</dt><dd>${repo.releases[repo.releases.length - 1].tag}</dd>`}
        ${d.default_branch && html`<dt>Default branch</dt><dd>${d.default_branch}</dd>`}
        ${d.language && html`<dt>Language</dt><dd>${d.language}</dd>`}
        ${d.license && html`<dt>Licence</dt><dd>${d.license}</dd>`}
        <dt>Stars · forks</dt><dd>${d.stars.toLocaleString()} · ${d.forks.toLocaleString()}</dd>
        ${d.created_at && html`<dt>Created</dt><dd>${day(d.created_at)}</dd>`}
        ${d.homepage && html`<dt>Website</dt><dd><a href=${d.homepage}>${d.homepage.replace(/^https?:\/\//, "")}</a></dd>`}
      </dl>
    </section>
    ${d.topics.length > 0 && html`<section><h2>Topics</h2><ul class="chips">${d.topics.map((t) => html`<li class="chip" key=${t}>${t}</li>`)}</ul></section>`}
    <section><h2>Your tags</h2><${Tags} repo=${repo} saveTags=${saveTags} /></section>
    <section><h2>Follow</h2><${Following} repo=${repo} save=${save} /></section>
  </aside>`;
}

export function RepoPage({ id, tab }) {
  const summary = useStore(ui, (s) => s.repos.find((r) => r.id === id));
  const [repo, setRepo] = useState(null);
  const [err, setErr] = useState("");
  const [snapshot, setSnapshot] = useState(null);
  const reload = () => api("repo", { id }).then((r) => { setRepo(r); setErr(""); }).catch((e) => setErr(errText(e)));
  // read the project again whenever a check changes it
  const stamp = summary ? `${summary.last_checked}|${summary.status}|${summary.snapshots}|${summary.releases}` : "";
  useEffect(() => { reload(); }, [id, stamp]);
  if (err && !repo) {
    return html`<div class="page"><p class="notice error">${err}</p><a class="button" href="#/">${Icon.back(14)} Back to the library</a></div>`;
  }
  if (!repo) return html`<div class="page"><p class="muted">Reading…</p></div>`;
  const s = repo.summary;
  const setTab = (t) => { location.hash = `#/repo/${id}${t === "readme" ? "" : `?tab=${t}`}`; };
  const remove = async () => {
    if (!(await confirmDialog({ title: `Remove ${s.name}?`, body: `Every captured version and release of ${id} (${bytes(s.bytes)}) is deleted from the library folder. This can't be undone.`, ok: "Remove from the library" }))) return;
    try { await api("remove", { id }); await loadRepos(); toast(`Removed ${id}`); location.hash = "#/"; } catch (e) { toast(errText(e), 6000); }
  };
  const tabs = [["readme", "Readme", null], ["versions", "Code versions", repo.snapshots.length], ["releases", "Releases", repo.releases.length]];
  return html`<div class="repo">
    <div class="repo-main">
      <header class="repo-head">
        <nav class="crumbs" aria-label="Where"><a href="#/">Library</a><span>/</span><span>${repo.source.host}</span><span>/</span><span>${s.owner}</span></nav>
        <div class="repo-title"><h1>${s.name}</h1>
          ${s.status === "working" && html`<span class="badge working">Downloading…</span>`}
          ${s.status === "queued" && html`<span class="badge queued">Waiting to be checked</span>`}
          ${s.archived && html`<span class="badge warn">archived upstream</span>`}
        </div>
        ${s.description && html`<p class="repo-desc">${s.description}</p>`}
        ${repo.last_error && html`<p class="notice error" id="repo-error">The last check didn't finish: ${repo.last_error}</p>`}
        <div class="repo-actions">
          <${LaunchControl} repo=${repo} onChange=${reload} />
          <button type="button" id="check-now" onClick=${() => check(id)} disabled=${s.status === "working" || s.status === "queued"}>${Icon.refresh(15)} Check now</button>
          <button type="button" onClick=${() => openFolder(id)} title="Open the project's folder">${Icon.folder(15)} Folder</button>
          <button type="button" onClick=${() => openUrl(repo.web_url)} title=${repo.web_url}>${Icon.external(15)} ${KIND[repo.source.kind] || "Website"}</button>
          <${Menu} id="repo-more" label=${Icon.dots(16)} title="More" align="left">
            <button type="button" onClick=${() => navigator.clipboard?.writeText(repo.web_url).then(() => toast("Link copied"))}>Copy the link</button>
            <div class="sep"></div>
            <button type="button" class="danger" id="remove" onClick=${remove}>${Icon.trash(14)} Remove from the library…</button>
          </${Menu}>
        </div>
      </header>
      <div class="tabs" role="tablist">${tabs.map(([k, l, n]) => html`<button type="button" role="tab" id=${"tab-" + k} aria-selected=${tab === k ? "true" : "false"} onClick=${() => setTab(k)}>${l}${n != null && html`<span class="n">${n}</span>`}</button>`)}</div>
      ${tab === "versions" ? html`<${Versions} repo=${repo} reload=${reload} showReadme=${(f) => { setSnapshot(f); setTab("readme"); }} />`
        : tab === "releases" ? html`<${Releases} repo=${repo} reload=${reload} />`
        : html`<${Readme} repo=${repo} snapshot=${snapshot} setSnapshot=${setSnapshot} />`}
    </div>
    <${Inspector} repo=${repo} reload=${reload} />
  </div>`;
}
