// A project's website: opening it on the built-in server, the files it loads from
// other servers saved for offline use, captures of the live site, building,
// comparing and exporting, and its settings (docs/PLAN.md, "Hosting websites").
import { html, useEffect, useRef, useState } from "../lib/html.js";
import { ui, api, ctx, toast, errText, poke, loadRepos } from "./state.js";
import { Icon } from "./icons.js";
import { confirmDialog } from "./dialogs.js";
import { openFolder, openUrl } from "./actions.js";
import { bytes, ago, when, day, plural } from "../lib/format.js";

const COLL = { code: "Code", pages: "Published branch", live: "Live capture" };

/** Open a project's site (or one version of it) in the browser. */
export async function openSite(id, coll = null, folder = null) {
  try {
    const r = await api("site_open", { id, coll, folder });
    ui.set({ info: { ...ui.get().info, hosting: r.port } });
    await openUrl(r.url);
    return r.url;
  } catch (e) {
    toast(errText(e), 7000);
    return null;
  }
}

export async function copySite(id, coll = null, folder = null) {
  try {
    const r = await api("site_open", { id, coll, folder });
    ui.set({ info: { ...ui.get().info, hosting: r.port } });
    await navigator.clipboard?.writeText(r.url);
    toast(`Copied ${r.url}`);
  } catch (e) {
    toast(errText(e), 7000);
  }
}

async function job(cmd, args, what) {
  try {
    const r = await api(cmd, args);
    toast(r.job ? what : "That's in hand already");
    poke();
    return true;
  } catch (e) {
    toast(errText(e), 7000);
    return false;
  }
}

/** The project's Open site button, beside Launch. */
export function SiteButton({ repo, primary }) {
  const s = repo.summary;
  if (!s.site) return null;
  return html`<button type="button" id="open-site" class=${primary ? "primary" : ""} title="Open the website in your browser, hosted by Repo Sync from the library"
    onClick=${() => openSite(s.id)}>${Icon.globe(15)} Open site</button>`;
}

function OfflineState({ v }) {
  const o = v.offline;
  if (!o) return html`<span class="muted">Not saved yet</span>`;
  const need = o.needs_internet || [];
  return html`<span title=${(o.hosts || []).join(", ")}>${plural(o.files, "file")} saved (${bytes(o.bytes)})</span>
    ${o.failed?.length > 0 && html` <span class="badge warn" title=${o.failed.map((f) => `${f.url}: ${f.why}`).join("\n")}>${o.failed.length} failed</span>`}
    ${need.length > 0 && html` <span class="badge plain" title=${need.map((n) => `${n.host} (${n.example})`).join("\n")}>${plural(need.length, "server")} need the internet</span>`}`;
}

function ExportForm({ id, v, close }) {
  const [format, setFormat] = useState("folder");
  const [parent, setParent] = useState("");
  const [name, setName] = useState(`${id.split("/").pop()}-${v.code}`);
  const [push, setPush] = useState("");
  const [workflow, setWorkflow] = useState(true);
  const pick = async () => {
    const p = await ctx().pickFolder("Where to save the export");
    if (p) setParent(p);
  };
  const go = async (e) => {
    e.preventDefault();
    if (!parent) { toast("Choose a folder to save it in"); return; }
    const sep = parent.includes("\\") && !parent.includes("/") ? "\\" : "/";
    const dest = `${parent.replace(/[\\/]+$/, "")}${sep}${name}${format === "zip" ? ".zip" : ""}`;
    const ok = await job("site_export", { id, coll: v.coll, folder: v.folder, options: { format: push ? "git" : format, dest, pages_workflow: workflow }, push: push.trim() }, `Exporting to ${dest}…`);
    if (ok) close();
  };
  return html`<form class="export" id="export-form" onSubmit=${go}>
    <div class="row">
      <label class="field"><span>As</span><select id="export-format" value=${format} onChange=${(e) => setFormat(e.target.value)}>
        <option value="folder">A folder</option><option value="zip">A ZIP file</option><option value="git">A Git repository (folder)</option></select></label>
      <label class="field" style="flex:1"><span>Name</span><input id="export-name" type="text" value=${name} onInput=${(e) => setName(e.target.value)} /></label>
    </div>
    <div class="row"><button type="button" id="export-pick" onClick=${pick}>${Icon.folder(14)} Save in…</button><span class="mono muted">${parent || "No folder chosen"}</span></div>
    <label class="check"><input type="checkbox" checked=${workflow} onChange=${(e) => setWorkflow(e.target.checked)} /> Add a GitHub Pages workflow, so the repo publishes the site</label>
    <label class="field"><span>Then push to an empty GitHub repo (optional)</span><input id="export-push" type="url" placeholder="https://github.com/you/name" value=${push} onInput=${(e) => setPush(e.target.value)} /></label>
    <p class="hint">The export holds the site's files, the files it loads from other servers in <span class="mono">vendor/</span> (with its addresses pointed there), and a README saying where it came from. Pushing uses your GitHub token from Settings; create the empty repo on GitHub first.</p>
    <div class="buttons"><button type="button" onClick=${close}>Cancel</button><button type="submit" class="primary" id="export-go">${Icon.box(14)} Export</button></div>
  </form>`;
}

function Compare({ id, a, b, close }) {
  const [res, setRes] = useState(null);
  const [err, setErr] = useState("");
  useEffect(() => { api("site_compare", { id, a: { coll: a.coll, folder: a.folder }, b: { coll: b.coll, folder: b.folder } }).then(setRes).catch((e) => setErr(errText(e))); }, [a.folder, b.folder]);
  return html`<div class="compare" id="compare">
    <div class="row"><b>${COLL[b.coll]} ${b.code}</b><span class="muted">compared with ${COLL[a.coll].toLowerCase()} ${a.code}</span><span class="spacer" style="flex:1"></span><button type="button" class="ghost small" onClick=${close}>${Icon.x(13)}</button></div>
    ${err ? html`<p class="notice error">${err}</p>` : !res ? html`<p class="muted">Comparing…</p>` : html`
      <p>${plural(res.added.length, "file")} added, ${res.removed.length} removed, ${res.changed.length} changed, ${res.same} the same.</p>
      ${res.pages.map((p) => html`<div class="page-change"><div class="mono">${p.path} <span class="muted">${p.title}</span></div>
        ${p.removed.map((l) => html`<div class="del">− ${l}</div>`)}${p.added.map((l) => html`<div class="add">+ ${l}</div>`)}</div>`)}
      ${res.added.length > 0 && html`<details><summary>Added</summary><ul class="mono small-list">${res.added.slice(0, 200).map((f) => html`<li>${f}</li>`)}</ul></details>`}
      ${res.removed.length > 0 && html`<details><summary>Removed</summary><ul class="mono small-list">${res.removed.slice(0, 200).map((f) => html`<li>${f}</li>`)}</ul></details>`}
      ${res.changed.length > 0 && html`<details><summary>Changed</summary><ul class="mono small-list">${res.changed.slice(0, 200).map((f) => html`<li>${f}</li>`)}</ul></details>`}`}
  </div>`;
}

function SiteVersions({ repo, info, reload }) {
  const id = repo.summary.id;
  const [open, setOpen] = useState(null); // { kind: "export" | "compare" | "log", folder }
  const list = info.versions;
  if (!list.length) return html`<p class="muted">Nothing has been captured yet.</p>`;
  const build = async (v) => {
    const plan = v.plan;
    if (!info.settings.build.approved) {
      const ok = await confirmDialog({
        title: "Build this site on this computer?",
        body: `Building runs the project's own code here: ${plan ? `“${plan.command}”` : "its build command"}, which can install packages and do anything a program can. Only build projects you trust. The build runs in a copy of the version, and the captured version isn't changed.`,
        ok: "Build it", danger: false,
      });
      if (!ok) return;
    }
    await job("site_build", { id, coll: v.coll, folder: v.folder, approve: true }, "Building the site…");
  };
  const del = async (v) => {
    if (!(await confirmDialog({ title: "Delete this capture?", body: `The ${COLL[v.coll].toLowerCase()} captured ${when(v.captured)} (${bytes(v.bytes)}) is deleted from the library.`, ok: "Delete" }))) return;
    try { await api("version_delete", { id, kind: v.coll === "code" ? "source" : v.coll, folder: v.folder }); reload(); loadRepos(); } catch (e) { toast(errText(e), 6000); }
  };
  const prevOf = (v) => list.slice(list.indexOf(v) + 1).find((x) => x.coll === v.coll);
  return html`<table class="list" id="site-versions">
    <thead><tr><th>Version</th><th>Captured</th><th>As a website</th><th>Offline</th><th></th></tr></thead>
    <tbody>${list.map((v) => html`<tr key=${v.coll + v.folder} data-folder=${v.folder} data-coll=${v.coll}>
      <td><span class="badge plain">${COLL[v.coll]}</span> <span class="mono">${v.code}</span> ${v.main && html`<span class="badge ok" title="Shown at the site's main address">main</span>`}
        <div class="sub">${v.message}</div></td>
      <td class="date" title=${when(v.captured)}>${day(v.captured)}</td>
      <td>${v.kind_label}${v.built ? " (built here)" : ""}${!v.servable && html`<div class="sub">${v.why_not}</div>`}</td>
      <td><${OfflineState} v=${v} /></td>
      <td class="num nowrap">
        <button type="button" class="ghost small" title=${v.servable ? "Open this version in the browser" : v.why_not} disabled=${!v.servable} data-act="open" onClick=${() => openSite(id, v.coll, v.folder)}>${Icon.globe(14)}</button>
        ${(v.kind === "build" || v.kind === "jekyll") && html`<button type="button" class="ghost small" data-act="build" title=${v.plan ? `Build with “${v.plan.command}”${v.plan.missing.length ? ` (needs ${v.plan.missing.join(", ")})` : ""}` : "No build command: set one in the settings below"} onClick=${() => build(v)}>${Icon.code(14)} Build</button>`}
        <button type="button" class="ghost small" data-act="offline" title="Save the files this version loads from other servers (scripts, fonts) so it works offline" disabled=${!v.servable} onClick=${() => job("site_offline", { id, coll: v.coll, folder: v.folder, refresh: false }, "Saving files for offline use…")}>${Icon.download(14)}</button>
        ${prevOf(v) && html`<button type="button" class="ghost small" data-act="compare" title="Compare with the capture before" onClick=${() => setOpen(open?.kind === "compare" && open.folder === v.folder ? null : { kind: "compare", folder: v.folder })}>${Icon.compare(14)}</button>`}
        <button type="button" class="ghost small" data-act="export" title="Export as a repo of its own" disabled=${!v.servable} onClick=${() => setOpen(open?.kind === "export" && open.folder === v.folder ? null : { kind: "export", folder: v.folder })}>${Icon.box(14)}</button>
        <button type="button" class="ghost small" title="Open this version's folder" onClick=${() => openFolder(id, `${v.coll === "code" ? "source" : v.coll}/${v.folder}`)}>${Icon.folder(14)}</button>
        ${v.coll !== "code" && html`<button type="button" class="ghost small danger" title="Delete this capture" onClick=${() => del(v)}>${Icon.trash(14)}</button>`}
      </td></tr>
      ${open?.folder === v.folder && html`<tr class="expand"><td colspan="5">
        ${open.kind === "export" ? html`<${ExportForm} id=${id} v=${v} close=${() => setOpen(null)} />`
          : html`<${Compare} id=${id} a=${prevOf(v)} b=${v} close=${() => setOpen(null)} />`}</td></tr>`}
      ${v.build_log && html`<tr class="expand"><td colspan="5"><details><summary>Build log${v.build ? ` · ${v.build.ok ? "built" : "failed"} ${ago(v.build.finished)}` : ""}</summary><pre class="log">${v.build_log}</pre></details></td></tr>`}`)}
    </tbody></table>`;
}

const listText = (a) => (a || []).join(", ");
const listOf = (t) => t.split(/[\s,]+/).map((x) => x.trim().toLowerCase()).filter(Boolean);

function SiteSettings({ repo, info, reload }) {
  const id = repo.summary.id;
  const [s, setS] = useState(info.settings);
  const [follow, setFollow] = useState({ live: info.follow_live, site_branch: info.follow_site_branch });
  const [busy, setBusy] = useState(false);
  useEffect(() => { setS(info.settings); setFollow({ live: info.follow_live, site_branch: info.follow_site_branch }); }, [JSON.stringify(info.settings), info.follow_live, info.follow_site_branch]);
  const set = (patch) => setS((x) => ({ ...x, ...patch }));
  const web = info.website;
  const save = async (e) => {
    e.preventDefault();
    setBusy(true);
    try {
      await api("site_set", { id, site: s });
      if (follow.live !== info.follow_live || follow.site_branch !== info.follow_site_branch) {
        await api("follow_set", { id, follow: { ...repo.follow, live: follow.live, site_branch: follow.site_branch } });
      }
      toast("Website settings saved");
      reload();
      loadRepos();
    } catch (e2) { toast(errText(e2), 7000); }
    setBusy(false);
  };
  return html`<form class="site-settings" id="site-settings" onSubmit=${save}>
    <div class="grid2">
      <label class="field"><span>Address name</span>
        <span class="addr"><input id="site-label" type="text" placeholder=${info.label} value=${s.label} onInput=${(e) => set({ label: e.target.value })} /><span class="muted">.localhost</span></span></label>
      <label class="field"><span>${web ? "Website address" : "Live address"}</span>
        <input id="site-live" type="url" placeholder=${info.live_url || "https://…"} value=${s.live_url} onInput=${(e) => set({ live_url: e.target.value })} /></label>
      ${!web && html`<label class="field"><span>Folder the site is in</span><input id="site-root" type="text" placeholder="as found" value=${s.root} onInput=${(e) => set({ root: e.target.value })} /></label>
      <label class="field"><span>Path it's served under</span><input id="site-base" type="text" placeholder="as found" value=${s.base} onInput=${(e) => set({ base: e.target.value })} /></label>
      <label class="field"><span>Kind</span><select id="site-kind" value=${s.kind} onChange=${(e) => set({ kind: e.target.value })}>
        <option value="auto">As found</option><option value="static">Plain files</option><option value="jekyll-passthrough">Jekyll from a branch (Markdown shown as pages)</option><option value="build">Built when published</option></select></label>`}
      <label class="field"><span>Unknown addresses</span><select id="site-spa" value=${s.spa_fallback} onChange=${(e) => set({ spa_fallback: e.target.value })}>
        <option value="auto">As found</option><option value="index">Get index.html (apps that route in the browser)</option><option value="none">Get 404</option></select></label>
    </div>
    <label class="check"><input type="checkbox" id="site-enabled" checked=${s.enabled !== false} onChange=${(e) => set({ enabled: e.target.checked ? (info.is_site ? null : true) : false })} /> Host this project as a website</label>
    <label class="check"><input type="checkbox" id="site-offline" checked=${s.offline} onChange=${(e) => set({ offline: e.target.checked })} /> Save the files it loads from other servers after each capture</label>
    <label class="check"><input type="checkbox" id="site-recording" checked=${s.recording} onChange=${(e) => set({ recording: e.target.checked })} /> While browsing, save files and pages it's missing</label>
    <label class="check"><input type="checkbox" id="site-block" checked=${s.block_internet} onChange=${(e) => set({ block_internet: e.target.checked })} /> Block the internet while it's open (to test that it works offline)</label>
    ${!web && html`<label class="check"><input type="checkbox" id="follow-live" checked=${follow.live} onChange=${(e) => setFollow({ ...follow, live: e.target.checked })} /> Capture the live site at each check</label>
    <label class="check"><input type="checkbox" checked=${follow.site_branch} onChange=${(e) => setFollow({ ...follow, site_branch: e.target.checked })} /> Capture the branch the site is published to${info.site_branch ? ` (${info.site_branch})` : ""}</label>`}
    <div class="grid2">
      <label class="field"><span>Other servers whose files are kept</span><input id="site-extra" type="text" placeholder="cdn.example.com" value=${listText(s.extra_hosts)} onChange=${(e) => set({ extra_hosts: listOf(e.target.value) })} /></label>
      <label class="field"><span>Servers never fetched from</span><input type="text" value=${listText(s.skip_hosts)} onChange=${(e) => set({ skip_hosts: listOf(e.target.value) })} /></label>
      ${(web || follow.live) && html`<label class="field"><span>Pages captured, at most</span><input type="number" min="1" max="5000" value=${s.mirror.max_pages} onInput=${(e) => set({ mirror: { ...s.mirror, max_pages: Number(e.target.value) || 400 } })} /></label>
      <label class="field"><span>Links followed from the start page</span><input type="number" min="0" max="50" value=${s.mirror.depth} onInput=${(e) => set({ mirror: { ...s.mirror, depth: Number(e.target.value) } })} /></label>`}
      ${!web && html`<label class="field"><span>Build command</span><input id="site-build-command" type="text" placeholder="as found" value=${s.build.command} onInput=${(e) => set({ build: { ...s.build, command: e.target.value } })} /></label>
      <label class="field"><span>Folder the build makes</span><input type="text" placeholder="as found" value=${s.build.output} onInput=${(e) => set({ build: { ...s.build, output: e.target.value } })} /></label>`}
    </div>
    ${(web || follow.live) && html`<label class="check"><input type="checkbox" checked=${s.mirror.same_folder} onChange=${(e) => set({ mirror: { ...s.mirror, same_folder: e.target.checked } })} /> Only pages under the start address's folder</label>`}
    ${s.build.approved && html`<p class="hint">You've agreed to build this project on this computer. <button type="button" class="ghost small" onClick=${() => set({ build: { ...s.build, approved: false } })}>Withdraw</button></p>`}
    <div class="buttons"><button type="submit" class="primary" id="site-save" disabled=${busy}>Save settings</button></div>
  </form>`;
}

function Requests({ id }) {
  const [log, setLog] = useState(null);
  const read = () => api("site_log", { id }).then(setLog).catch(() => setLog([]));
  useEffect(() => { read(); const t = setInterval(read, 4000); return () => clearInterval(t); }, [id]);
  if (!log) return null;
  const shown = [...log].reverse().slice(0, 60);
  return html`<section><h2 class="section-head">Recent requests <span class="n">${log.length}</span></h2>
    ${!shown.length ? html`<p class="muted">Nothing yet. Open the site and its requests show here: what was saved from the internet, what it reached for, and what was missing.</p>`
      : html`<table class="list" id="site-log"><tbody>${shown.map((e, i) => html`<tr key=${i} class=${e.status >= 400 || e.method === "CSP" ? "warn" : ""}>
          <td class="date">${new Date(e.time).toLocaleTimeString()}</td><td class="mono">${e.method === "CSP" ? "internet" : e.status}</td>
          <td class="mono path">${e.path}</td><td class="muted">${e.note}</td></tr>`)}</tbody></table>`}
  </section>`;
}

export function SiteTab({ repo, reload: reloadRepo }) {
  const id = repo.summary.id;
  const [info, setInfo] = useState(null);
  const [err, setErr] = useState("");
  const read = () => api("site_info", { id }).then((i) => { setInfo(i); setErr(""); }).catch((e) => setErr(errText(e)));
  const stamp = `${repo.summary.last_checked}|${repo.summary.status}|${repo.snapshots.length}|${repo.pages?.length}|${repo.live?.length}`;
  useEffect(() => { read(); }, [id, stamp]);
  const reload = () => { read(); reloadRepo(); };
  if (err) return html`<p class="notice error">${err}</p>`;
  if (!info) return html`<p class="muted">Reading…</p>`;
  const main = info.versions.find((v) => v.main);
  const capture = () => job("site_capture", { id }, info.website ? "Capturing the website…" : "Capturing the live site…");
  return html`<div class="site" id="site">
    <section class="site-head">
      ${main ? html`<div class="row">
          <div><div class="site-kind">${main.kind_label}${main.built ? " (built here)" : ""}</div>
            <div class="mono muted" id="site-address">${info.url || `${info.label}.localhost`}</div></div>
          <span class="spacer" style="flex:1"></span>
          <button type="button" class="primary" id="site-open" onClick=${() => openSite(id)}>${Icon.globe(15)} Open site</button>
          <button type="button" onClick=${() => copySite(id)} title="Copy the site's address">Copy address</button>
          <button type="button" id="site-capture" onClick=${capture} disabled=${!info.live_url && !info.website} title=${info.live_url ? `Save ${info.live_url} as it is now` : "Set the live address in the settings below"}>${Icon.download(15)} ${info.website ? "Capture now" : "Capture live site"}</button>
        </div>
        ${(main.evidence || []).length > 0 && html`<ul class="evidence">${main.evidence.map((e) => html`<li>${e}</li>`)}</ul>`}
        ${(main.warnings || []).map((w) => html`<p class="notice">${w}</p>`)}
        ${!info.versions.some((v) => v.offline) && html`<p class="hint">Files from other servers (CDNs, fonts) haven't been saved yet: the site loads them from the internet. Use ${Icon.download(12)} on a version below, or check the project again.</p>`}`
      : html`<p class="notice">${info.is_site ? "No version of this project can be shown as a website yet." : "No website was found in this project."}
          ${info.versions[0]?.why_not ? ` ${info.versions[0].why_not}` : ""}</p>
        <div class="row">${info.live_url && html`<button type="button" id="site-capture" onClick=${capture}>${Icon.download(15)} Capture the live site (${info.live_url.replace(/^https?:\/\//, "")})</button>`}</div>`}
    </section>
    <h2 class="section-head">Versions <span class="n">${info.versions.length}</span></h2>
    <${SiteVersions} repo=${repo} info=${info} reload=${reload} />
    <${Requests} id=${id} />
    <h2 class="section-head">Settings</h2>
    <${SiteSettings} repo=${repo} info=${info} reload=${reload} />
  </div>`;
}

/** Every hosted site, with the server's address. */
export function SitesPage() {
  const [data, setData] = useState(null);
  const [err, setErr] = useState("");
  useEffect(() => {
    api("sites").then((d) => { setData(d); ui.set({ info: { ...ui.get().info, hosting: d.port } }); }).catch((e) => setErr(errText(e)));
  }, []);
  return html`<div class="cmdbar"><h1>Websites</h1>${data && html`<span class="count">${data.sites.length}</span>`}<span class="spacer"></span>
      ${data && html`<button type="button" onClick=${() => openUrl(data.index)}>${Icon.external(15)} Every site's address</button>`}</div>
    <div class="page">
      ${err ? html`<p class="notice error">${err}</p>` : !data ? html`<p class="muted">Starting the web server…</p>` : html`
        <p class="hint">Repo Sync hosts these from the library at <span class="mono">${data.index}</span>, each at its own address, for this computer only. Websites you add (Add, then a web address) and projects with a website are listed.</p>
        ${data.sites.length === 0 ? html`<p class="muted">No websites yet.</p>` : html`<table class="list" id="sites"><tbody>
          ${data.sites.map((s) => html`<tr key=${s.label}><td><b>${s.title}</b><div class="sub mono">${s.url}</div></td>
            <td class="num"><button type="button" class="small" onClick=${() => openUrl(s.url)}>${Icon.globe(13)} Open</button></td></tr>`)}</tbody></table>`}`}
    </div>`;
}
