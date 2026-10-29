// About: the version, what the app does, where it keeps things, and the licences.
import { html, useEffect, useState } from "../lib/html.js";
import { useStore } from "../lib/store.js";
import { ui } from "./state.js";
import { Mark } from "./icons.js";

export function AboutPage() {
  const info = useStore(ui, (s) => s.info);
  const [third, setThird] = useState("");
  const [build, setBuild] = useState(null);
  useEffect(() => {
    fetch("THIRD_PARTY.md").then((r) => (r.ok ? r.text() : "")).then(setThird).catch(() => {});
    fetch("build-info.json").then((r) => (r.ok ? r.json() : null)).then(setBuild).catch(() => {});
  }, []);
  return html`<div class="cmdbar"><h1>About</h1></div>
    <div class="page page-narrow about">
      <div class="panel">
        <div style="display:flex;gap:14px;align-items:center"><span style="width:48px;height:48px;display:inline-flex"><${Mark} /></span>
          <div><h2>Repo Sync ${info.version}</h2><p class="muted">${build ? `Built ${build.date}${build.commit ? ` from ${build.commit}` : ""}` : ""}</p></div></div>
        <p>A library of software projects that works offline. Add a project by its link (GitHub, GitLab, Codeberg and other Gitea or Forgejo servers): Repo Sync downloads its code and its latest release, checks for new ones on a schedule, and keeps every version beside the last. Each project's readme and details are shown from the copies on disk, and the latest release can be started from here.</p>
        <p><a href="https://github.com/DrFlGd/Repo-Sync">github.com/DrFlGd/Repo-Sync</a> · <a href="https://github.com/DrFlGd/Repo-Sync/releases">Releases</a></p>
      </div>
      ${third && html`<h2 class="section-head">Third-party code</h2><pre>${third}</pre>`}
    </div>`;
}
