// Dialogs: Add projects (one link or many), and asking before something is removed.
import { html, useEffect, useRef, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, api, toast, errText, poke, fileChoices } from "./state.js";
import { Icon } from "./icons.js";

export const openAdd = (text = "") => ui.set({ dialog: { type: "add", text } });

/** Ask before doing something that can't be undone. Resolves true to go ahead. */
export function confirmDialog({ title, body, ok = "Remove", danger = true }) {
  return new Promise((resolve) => ui.set({ dialog: { type: "confirm", title, body, ok, danger, resolve } }));
}


function AddDialog({ text }) {
  const [value, setValue] = useState(text || "");
  const [checks, setChecks] = useState([]);
  const [follow, setFollow] = useState(null);
  const [busy, setBusy] = useState(false);
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
  const add = async (e) => {
    e?.preventDefault();
    if (!good.length || busy) return;
    setBusy(true);
    setErr("");
    try {
      const res = await api("add", { inputs: good.map((c) => c.input), follow });
      const queued = res.filter((r) => r.job);
      ui.set({ dialog: null });
      toast(queued.length ? `Adding ${queued.length === 1 ? queued[0].id : `${queued.length} projects`}…` : "Nothing new to add");
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
      <textarea id="add-links" ref=${box} rows="4" placeholder=${"https://github.com/owner/name\nhttps://gitlab.com/group/project\nhttps://codeberg.org/owner/name"}
        value=${value} onInput=${(e) => setValue(e.target.value)}
        onKeyDown=${(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) add(e); }}></textarea></label>
    ${checks.length > 0 && html`<ul class="checks" aria-label="What the links name">${checks.map((c) => html`<li>
      ${!c.ok ? html`<span class="bad">${Icon.x(12)}</span><span class="bad">${c.error}</span>`
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
      <p class="hint">New versions are captured on the library's schedule (Settings), each beside the ones before.</p>
    </div>`}
    ${err && html`<p class="notice error">${err}</p>`}
    <div class="buttons">
      <button type="button" onClick=${() => ui.set({ dialog: null })}>Cancel</button>
      <button type="submit" class="primary" id="add-go" disabled=${!good.length || busy}>${Icon.plus(15)} ${good.length > 1 ? `Add ${good.length} projects` : "Add"}</button>
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
    ${d.type === "add" ? html`<${AddDialog} text=${d.text} />` : html`<${ConfirmDialog} d=${d} />`}
  </div>`;
}
