// The Activity page: the download in hand (with Stop), what's waiting, and what
// the last checks found.
import { html } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui, api, toast, errText, poke } from "./state.js";
import { Icon } from "./icons.js";
import { checkAll } from "./actions.js";
import { bytes, ago, when } from "../lib/format.js";

const project = (repo) => repo.startsWith("account:")
  ? html`<span>${repo.slice(8)}</span>`
  : html`<a href=${`#/repo/${repo}`}>${repo.split("/").slice(1).join("/")}</a>`;
const what = (j) => (j.type === "add" ? "Adding"
  : j.type === "account" ? (j.by === "schedule" ? "New projects of" : "Projects of")
  : j.type === "release" ? `Release ${j.tag}`
  : j.by === "schedule" ? "Scheduled check" : "Check");

async function stop(job) {
  try { await api("job_stop", { job }); poke(); } catch (e) { toast(errText(e), 6000); }
}

export function ActivityPage() {
  const jobs = useStore(ui, (s) => s.jobs);
  const next = useStore(ui, (s) => s.totals.next_check);
  const cur = jobs.current;
  const pct = cur && cur.total > 0 ? Math.min(100, (cur.done / cur.total) * 100) : null;
  return html`<div class="cmdbar"><h1>Activity</h1><span class="spacer"></span>
      ${next && html`<span class="muted" title=${when(next)}>Next scheduled check ${ago(next)}</span>`}
      <button type="button" onClick=${checkAll}>${Icon.refresh(15)} Check all now</button></div>
    <div class="page page-narrow">
      <h2 class="section-head">Now</h2>
      <div class="panel job-now" id="job-now">${cur ? html`
        <div><b>${what(cur.job)}</b> ${project(cur.job.repo)} <span class="muted">· started ${ago(cur.started)}</span></div>
        <div class="stage">${cur.stage}${pct !== null ? ` · ${bytes(cur.done)} of ${bytes(cur.total)}` : cur.done ? ` · ${bytes(cur.done)}` : ""}</div>
        <div class=${"meter big" + (pct === null ? " busy" : "")}><i style=${pct === null ? "" : `width:${pct}%`}></i></div>
        <div><button type="button" class="small" id="stop" onClick=${() => stop(cur.job.id)}>${Icon.x(13)} Stop</button></div>`
        : html`<p class="muted">Nothing is downloading.</p>`}</div>
      ${jobs.queued?.length > 0 && html`<h2 class="section-head">Waiting <span class="n">${jobs.queued.length}</span></h2>
        <ul class="log">${jobs.queued.map((j) => html`<li key=${j.id}><span class="t">${what(j)}</span><span>${project(j.repo)}
          <button type="button" class="ghost small" style="float:right" onClick=${() => stop(j.id)}>Cancel</button></span></li>`)}</ul>
        <p></p>`}
      <h2 class="section-head">Finished <span class="n">${jobs.recent?.length || 0}</span></h2>
      ${jobs.recent?.length ? html`<ul class="log" id="finished">${jobs.recent.map((f) => html`<li key=${f.job.id}>
        <span class="t" title=${when(f.finished)}>${ago(f.finished)}<br />${what(f.job)}</span>
        <span>${project(f.job.repo)}<br /><span class=${!f.ok ? "err" : f.outcome && (f.outcome.snapshot || f.outcome.release || f.outcome.note?.includes(" to add")) ? "new" : "muted"}>${f.message}</span></span>
      </li>`)}</ul>` : html`<p class="muted">Nothing has been checked since the app started.</p>`}
    </div>`;
}
