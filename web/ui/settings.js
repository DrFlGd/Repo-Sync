// Settings: the library folder, the schedule, what new projects follow, limits,
// tokens for the servers, and the theme.
import { html, useEffect, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, api, ctx, toast, errText, setPref, loadRepos, fileChoices, KEEP, keepLabel } from "./state.js";
import { Icon } from "./icons.js";
import { ago, when } from "../lib/format.js";

const EVERY = [[0, "Only by hand"], [1, "Every hour"], [3, "Every 3 hours"], [6, "Every 6 hours"], [12, "Every 12 hours"], [24, "Every day"], [168, "Every week"]];
const HOSTS = [["github.com", "GitHub"], ["gitlab.com", "GitLab"], ["codeberg.org", "Codeberg"]];

function Token({ host, label, set: isSet, onSaved }) {
  const [v, setV] = useState("");
  const save = async (value) => {
    try { await api("settings_set", { tokens: { [host]: value } }); setV(""); onSaved(); toast(value ? `Token for ${label} saved` : `Token for ${label} removed`); }
    catch (e) { toast(errText(e), 6000); }
  };
  return html`<div class="row">
    <span style="width:90px">${label}</span>
    <input type="password" autocomplete="off" style="flex:1" placeholder=${isSet ? "A token is saved" : "No token"} value=${v} onInput=${(e) => setV(e.target.value)} aria-label=${`Token for ${label}`} />
    <button type="button" class="small" disabled=${!v.trim()} onClick=${() => save(v.trim())}>Save</button>
    ${isSet && html`<button type="button" class="small ghost" onClick=${() => save("")}>Remove</button>`}
  </div>`;
}

// Accounts followed for their new projects (added from a link to a user or organisation).
function Accounts() {
  const [list, setList] = useState(null);
  const load = () => api("accounts").then(setList).catch(() => setList([]));
  useEffect(() => { load(); }, []);
  const stop = async (id) => {
    try { await api("account_remove", { id }); load(); toast(`No longer following ${id}`); } catch (e) { toast(errText(e), 6000); }
  };
  return html`<section><h2 class="section-head">Followed accounts</h2><div class="panel" id="accounts">
    <p class="hint">Add a link to a user or organisation (like https://github.com/someone) to add all of their projects. A followed account's new projects are added at each scheduled check. Projects you remove aren't added back.</p>
    ${!list ? null : !list.length ? html`<p class="muted">No account is followed.</p>`
      : list.map((a) => html`<div class="row" key=${a.id} data-account=${a.id}>
        <a class="mono" style="flex:1" href=${a.web_url}>${a.id}</a>
        <span class="muted">${a.last_checked ? `looked ${ago(a.last_checked)}` : ""}</span>
        <button type="button" class="small ghost" onClick=${() => stop(a.id)}>Stop following</button></div>`)}
  </div></section>`;
}

export function SettingsPage() {
  const info = useStore(ui, (s) => s.info);
  const theme = useStore(ui, (s) => s.theme);
  const [s, setS] = useState(null);
  const load = () => api("settings_get").then(setS).catch(() => setS(null));
  useEffect(() => { load(); }, [info.library?.path]);
  const saveLib = async (patch) => {
    try { setS(await api("settings_set", { library: { ...s.library, ...patch } })); loadRepos(); } catch (e) { toast(errText(e), 6000); }
  };
  const setNew = (patch) => saveLib({ new_follow: { ...s.library.new_follow, ...patch } });
  const change = async () => {
    const p = await ctx().pickFolder("Choose the library folder");
    if (!p) return;
    try {
      const i = await api("library_open", { path: p });
      ui.set({ info: i, filter: null });
      await loadRepos();
      toast(`Library: ${p}`);
    } catch (e) { toast(errText(e), 6000); }
  };
  const lib = s?.library;
  return html`<div class="cmdbar"><h1>Settings</h1></div>
    <div class="page"><div class="settings">
      <section><h2 class="section-head">Library</h2><div class="panel">
        <div class="row"><span class="path" id="library-path">${info.library?.path || "No library open"}</span>
          <button type="button" id="change-library" onClick=${change}>Change…</button>
          ${info.library && html`<button type="button" onClick=${() => ctx().openPath(info.library.path)}>${Icon.folder(15)} Open</button>`}</div>
        <p class="hint">Each project gets a folder in it (host / owner / name) holding every captured version of its code under <span class="mono">source/</span> and every captured release under <span class="mono">releases/</span>, with its details in <span class="mono">repo.json</span>. The folder works without the app and can be moved, synced or opened on another computer.</p>
      </div></section>
      ${lib && html`<section><h2 class="section-head">Updates</h2><div class="panel">
        <label class="field"><span>Check every project</span>
          <select id="check-every" value=${String(lib.check_every_hours)} onChange=${(e) => saveLib({ check_every_hours: Number(e.target.value) })}>
            ${EVERY.map(([v, l]) => html`<option value=${String(v)}>${l}</option>`)}
            ${!EVERY.some(([v]) => v === lib.check_every_hours) && html`<option value=${String(lib.check_every_hours)}>Every ${lib.check_every_hours} hours</option>`}</select></label>
        <p class="hint">Checks run while Repo Sync is open (one download at a time); a project can have its own schedule on its page. New versions and releases are added beside the ones already kept; nothing is deleted unless you set a limit below.
          ${s.next_check && html` Next check ${ago(s.next_check)} (${when(s.next_check)}).`}</p>
        <p class="hint">To check while the app is closed, schedule <span class="mono">reposync-cli check-all</span> (for example with Windows Task Scheduler).</p>
        <div class="row" style="gap:16px; flex-wrap:wrap">
          <label class="field" style="flex:1"><span>Versions of the code to keep</span>
            <select id="keep-source" value=${String(lib.keep_source || 0)} onChange=${(e) => saveLib({ keep_source: Number(e.target.value) })}>
              ${KEEP.map(([v]) => html`<option value=${String(v)}>${keepLabel(v)}</option>`)}
              ${!KEEP.some(([v]) => v === (lib.keep_source || 0)) && html`<option value=${String(lib.keep_source)}>${keepLabel(lib.keep_source)}</option>`}</select></label>
          <label class="field" style="flex:1"><span>Releases to keep</span>
            <select id="keep-releases" value=${String(lib.keep_releases || 0)} onChange=${(e) => saveLib({ keep_releases: Number(e.target.value) })}>
              ${KEEP.map(([v]) => html`<option value=${String(v)}>${keepLabel(v)}</option>`)}
              ${!KEEP.some(([v]) => v === (lib.keep_releases || 0)) && html`<option value=${String(lib.keep_releases)}>${keepLabel(lib.keep_releases)}</option>`}</select></label>
        </div>
        <p class="hint">With a limit, each check deletes the oldest versions or releases past it, after capturing anything new. Releases you ticked yourself in a project's Releases tab are always kept. A project can have its own limits on its page.</p>
        <label class="field"><span>Don't download release files bigger than</span>
          <div class="row"><input type="number" min="0" step="50" style="width:120px" value=${lib.max_file_mb} onChange=${(e) => saveLib({ max_file_mb: Math.max(0, Math.round(Number(e.target.value) || 0)) })} /> MB <span class="hint">(0: no limit)</span></div></label>
      </div></section>
      <section><h2 class="section-head">New projects follow</h2><div class="panel">
        <label class="check"><input type="checkbox" checked=${lib.new_follow.source} onChange=${(e) => setNew({ source: e.target.checked })} /> Their code (default branch)</label>
        <label class="check"><input type="checkbox" checked=${lib.new_follow.releases} onChange=${(e) => setNew({ releases: e.target.checked })} /> Their latest release</label>
        <label class="check"><input type="checkbox" checked=${lib.new_follow.prereleases} onChange=${(e) => setNew({ prereleases: e.target.checked })} /> including pre-releases</label>
        <label class="field"><span>Release files to keep</span>
          <select value=${lib.new_follow.files} onChange=${(e) => setNew({ files: e.target.value })}>
            ${fileChoices().map(([k, l]) => html`<option value=${k}>${l}</option>`)}</select></label>
      </div></section>
      <section><h2 class="section-head">Tokens</h2><div class="panel">
        <p class="hint">Without a token GitHub allows 60 requests an hour (a check takes 3 or 4), and private projects can't be reached. A token with read access raises that to 5,000. Tokens stay on this computer, never in the library.</p>
        ${HOSTS.map(([h, l]) => html`<${Token} key=${h} host=${h} label=${l} set=${!!s.tokens[h]} onSaved=${load} />`)}
      </div></section>`}
      ${s && html`<${Accounts} />`}
      <section><h2 class="section-head">Appearance</h2><div class="panel">
        <label class="field"><span>Theme</span>
          <select id="theme" value=${theme} onChange=${(e) => setPref({ theme: e.target.value })}>
            <option value="system">Follow the system</option><option value="light">Light</option><option value="dark">Dark</option></select></label>
      </div></section>
    </div></div>`;
}
