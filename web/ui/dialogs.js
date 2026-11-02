// Dialogs: Add projects (one link or many), asking before something is removed,
// asking for a name, and choosing the projects in a collection.
import { html, useEffect, useRef, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, api, toast, errText, poke, fileChoices } from "./state.js";
import { Icon } from "./icons.js";

export const openAdd = (text = "") => ui.set({ dialog: { type: "add", text } });

/** Ask before doing something that can't be undone. Resolves true to go ahead. */
export function confirmDialog({ title, body, ok = "Remove", danger = true }) {
  return new Promise((resolve) => ui.set({ dialog: { type: "confirm", title, body, ok, danger, resolve } }));
}

/** Ask for a name (a new collection, a new name). Resolves the trimmed text, or null.
 * `check(text)` may throw to keep the dialog open with its message. */
export function promptDialog({ title, label, value = "", ok = "Save", placeholder = "", check }) {
  return new Promise((resolve) => ui.set({ dialog: { type: "prompt", title, label, value, ok, placeholder, check, resolve: (v) => resolve(v || null) } }));
}

/** Tick the projects a collection holds. Resolves { add: [ids], remove: [ids] } or null. */
export function chooseDialog({ title, chosen }) {
  return new Promise((resolve) => ui.set({ dialog: { type: "choose", title, chosen, resolve: (v) => resolve(v || null) } }));
}

function PromptDialog({ d }) {
  const [text, setText] = useState(d.value);
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const box = useRef(null);
  useEffect(() => { box.current?.focus(); box.current?.select(); }, []);
  const go = async (e) => {
    e.preventDefault();
    const v = text.trim();
    if (!v || busy) return;
    setBusy(true);
    try {
      await d.check?.(v);
      ui.set({ dialog: null });
      d.resolve(v);
    } catch (e2) {
      setErr(errText(e2));
      setBusy(false);
    }
  };
  return html`<form class="dialog narrow" role="dialog" aria-modal="true" aria-labelledby="prompt-title" onSubmit=${go}>
    <h2 id="prompt-title">${d.title}</h2>
    <label class="field"><span>${d.label}</span>
      <input id="prompt-input" ref=${box} type="text" maxlength="80" placeholder=${d.placeholder} value=${text} onInput=${(e) => { setText(e.target.value); setErr(""); }} /></label>
    ${err && html`<p class="notice error">${err}</p>`}
    <div class="buttons">
      <button type="button" onClick=${() => { ui.set({ dialog: null }); d.resolve(null); }}>Cancel</button>
      <button type="submit" class="primary" id="prompt-ok" disabled=${!text.trim() || busy}>${d.ok}</button>
    </div>
  </form>`;
}

function ChooseDialog({ d }) {
  const repos = useStore(ui, (s) => s.repos);
  const [picked, setPicked] = useState(() => new Set(d.chosen));
  const [find, setFind] = useState("");
  const box = useRef(null);
  useEffect(() => box.current?.focus(), []);
  const words = find.toLowerCase().split(/\s+/).filter(Boolean);
  const list = [...repos]
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }))
    .filter((r) => words.every((w) => `${r.id} ${r.name} ${r.description}`.toLowerCase().includes(w)));
  const flip = (id, on) => setPicked((p) => { const n = new Set(p); on ? n.add(id) : n.delete(id); return n; });
  const before = new Set(d.chosen);
  const add = [...picked].filter((id) => !before.has(id));
  const remove = d.chosen.filter((id) => !picked.has(id));
  const save = (e) => { e.preventDefault(); ui.set({ dialog: null }); d.resolve({ add, remove }); };
  return html`<form class="dialog" role="dialog" aria-modal="true" aria-labelledby="choose-title" onSubmit=${save}>
    <h2 id="choose-title">${d.title}</h2>
    <label class="search wide"><span class="visually-hidden">Find a project</span>${Icon.search(15)}
      <input id="choose-find" ref=${box} type="search" placeholder="Find a project" value=${find} onInput=${(e) => setFind(e.target.value)} /></label>
    <ul class="choose" id="choose-list">${list.map((r) => html`<li key=${r.id}><label class="check">
      <input type="checkbox" data-id=${r.id} checked=${picked.has(r.id)} onChange=${(e) => flip(r.id, e.target.checked)} />
      <span class="name">${r.name}</span><span class="muted mono">${r.host} / ${r.owner}</span></label></li>`)}
      ${list.length === 0 && html`<li class="muted">${repos.length ? "Nothing matches." : "The library has no projects yet."}</li>`}</ul>
    <p class="hint">${picked.size} ${picked.size === 1 ? "project" : "projects"} ticked. A project can be in any number of collections; taking it out of one doesn't remove it from the library.</p>
    <div class="buttons">
      <button type="button" onClick=${() => { ui.set({ dialog: null }); d.resolve(null); }}>Cancel</button>
      <button type="submit" class="primary" id="choose-ok" disabled=${!add.length && !remove.length}>Save</button>
    </div>
  </form>`;
}

function AddDialog({ text }) {
  const [value, setValue] = useState(text || "");
  const [checks, setChecks] = useState([]);
  const [follow, setFollow] = useState(null);
  const [busy, setBusy] = useState(false);
  const [watch, setWatch] = useState(true);
  const [forks, setForks] = useState(false);
  const [err, setErr] = useState("");
  const box = useRef(null);
  useEffect(() => {
    box.current?.focus();
    api("settings_get").then((s) => setFollow({ ...s.library.new_follow })).catch(() => setFollow({ source: true, releases: true, prereleases: false, files: "all", branch: "", every_hours: null }));
  }, []);
  // what each line names, as it's typed (no network: the server is asked when it's added)
  useEffect(() => {
    if (!value.trim()) { setChecks([]); return; }
    const t = setTimeout(() => api("link_check", { input: value }).then(setChecks).catch(() => {}), 120);
    return () => clearTimeout(t);
  }, [value]);
  const good = checks.filter((c) => c.ok && !c.in_library);
  const accounts = good.filter((c) => c.account);
  const add = async (e) => {
    e?.preventDefault();
    if (!good.length || busy) return;
    setBusy(true);
    setErr("");
    try {
      const res = await api("add", { inputs: good.map((c) => c.input), follow, watch, forks });
      const queued = res.filter((r) => r.job && r.id);
      const listing = res.filter((r) => r.job && r.account);
      ui.set({ dialog: null });
      toast(listing.length ? `Listing the projects of ${listing.map((r) => r.account).join(", ")}…`
        : queued.length ? `Adding ${queued.length === 1 ? queued[0].id : `${queued.length} projects`}…` : "Nothing new to add");
      poke();
      if (location.hash.startsWith("#/repo/") || location.hash.startsWith("#/settings")) location.hash = "#/";
    } catch (e2) {
      setErr(errText(e2));
      setBusy(false);
    }
  };
  const set = (patch) => setFollow((f) => ({ ...f, ...patch }));
  return html`<form class="dialog" role="dialog" aria-modal="true" aria-labelledby="add-title" onSubmit=${add}>
    <h2 id="add-title">Add projects</h2>
    <label class="field"><span>Links, one per line</span>
      <textarea id="add-links" ref=${box} rows="4" placeholder=${"https://github.com/owner/name\nhttps://gitlab.com/group/project\nhttps://github.com/someone (all of their projects)"}
        value=${value} onInput=${(e) => setValue(e.target.value)}
        onKeyDown=${(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) add(e); }}></textarea></label>
    ${checks.length > 0 && html`<ul class="checks" aria-label="What the links name">${checks.map((c) => html`<li>
      ${!c.ok ? html`<span class="bad">${Icon.x(12)}</span><span class="bad">${c.error}</span>`
        : c.account ? html`<span class="good">${Icon.check(12)}</span><span class="mono">${c.account.id}</span><span class="muted">every project of this ${c.account.kind} account${c.following ? " (followed already)" : ""}</span>`
        : c.in_library ? html`<span class="have">${Icon.alert(12)}</span><span class="mono">${c.id}</span><span class="have">already in the library</span>`
        : html`<span class="good">${Icon.check(12)}</span><span class="mono">${c.id}</span><span class="muted">${c.kind || "server found when added"}</span>`}
    </li>`)}</ul>`}
    ${follow && html`<div class="options">
      <div class="row">
        <label class="check"><input type="checkbox" id="add-source" checked=${follow.source} onChange=${(e) => set({ source: e.target.checked })} /> The code (default branch)</label>
        <label class="check"><input type="checkbox" id="add-releases" checked=${follow.releases} onChange=${(e) => set({ releases: e.target.checked })} /> The newest release</label>
        <label class="check"><input type="checkbox" checked=${follow.prereleases} disabled=${!follow.releases} onChange=${(e) => set({ prereleases: e.target.checked })} /> including pre-releases</label>
      </div>
      <div class="row"><label class="field" style="flex:1"><span>Release files to keep</span>
        <select id="add-files" value=${follow.files} disabled=${!follow.releases} onChange=${(e) => set({ files: e.target.value })}>${fileChoices().map(([k, l]) => html`<option value=${k}>${l}</option>`)}</select></label></div>
      ${accounts.length > 0 && html`<div class="row">
        <label class="check"><input type="checkbox" id="add-watch" checked=${watch} onChange=${(e) => setWatch(e.target.checked)} /> Add their new projects at each check</label>
        <label class="check"><input type="checkbox" id="add-forks" checked=${forks} onChange=${(e) => setForks(e.target.checked)} /> Include their forks of other projects</label>
      </div>`}
      <p class="hint">New versions are captured on the library's schedule (Settings), each beside the ones before.</p>
    </div>`}
    ${err && html`<p class="notice error">${err}</p>`}
    <div class="buttons">
      <button type="button" onClick=${() => ui.set({ dialog: null })}>Cancel</button>
      <button type="submit" class="primary" id="add-go" disabled=${!good.length || busy}>${Icon.plus(15)} ${accounts.length ? (accounts.length === good.length && accounts.length === 1 ? "Add their projects" : `Add ${good.length} links`) : good.length > 1 ? `Add ${good.length} projects` : "Add"}</button>
    </div>
  </form>`;
}

function ConfirmDialog({ d }) {
  const close = (v) => { ui.set({ dialog: null }); d.resolve(v); };
  const okRef = useRef(null);
  useEffect(() => okRef.current?.focus(), []);
  return html`<div class="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title">
    <h2 id="confirm-title">${d.title}</h2>
    <p>${d.body}</p>
    <div class="buttons">
      <button type="button" onClick=${() => close(false)}>Cancel</button>
      <button type="button" id="confirm-ok" ref=${okRef} class=${d.danger ? "danger" : "primary"} onClick=${() => close(true)}>${d.ok}</button>
    </div>
  </div>`;
}

export function Dialogs() {
  const d = useStore(ui, (s) => s.dialog);
  if (!d) return null;
  const dismiss = (e) => { if (e.target === e.currentTarget) { if (d.resolve) d.resolve(false); ui.set({ dialog: null }); } };
  return html`<div class="backdrop" onMouseDown=${dismiss}>
    ${d.type === "add" ? html`<${AddDialog} text=${d.text} />`
      : d.type === "prompt" ? html`<${PromptDialog} d=${d} />`
      : d.type === "choose" ? html`<${ChooseDialog} d=${d} />`
      : html`<${ConfirmDialog} d=${d} />`}
  </div>`;
}
