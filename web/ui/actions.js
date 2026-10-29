// Actions more than one page offers: launching, checking, opening folders and links.
import { api, ctx, toast, errText, poke } from "./state.js";

export async function launch(id, { folder = null, file = "", remember = false } = {}) {
  try {
    const r = await api("launch", { id, folder, file, remember });
    toast(`Started ${r.file.split("/").pop()} from ${r.folder}`);
    return r;
  } catch (e) {
    toast(errText(e), 6000);
    return null;
  }
}

export async function check(id) {
  try {
    const r = await api("check", { id });
    toast(r.job ? "Checking for updates…" : "Already checking this project");
    poke();
  } catch (e) {
    toast(errText(e), 6000);
  }
}

export async function checkAll() {
  try {
    const r = await api("check_all");
    toast(r.queued ? `Checking ${r.queued} ${r.queued === 1 ? "project" : "projects"} for updates…` : "Everything is being checked already");
    poke();
  } catch (e) {
    toast(errText(e), 6000);
  }
}

/** Open a project's folder (or one of its versions: sub = "source/<folder>") in the file manager. */
export async function openFolder(id, sub = "") {
  try {
    const p = await api("path", { id, sub });
    await ctx().openPath(p);
  } catch (e) {
    toast(errText(e), 6000);
  }
}

export const openUrl = (url) => ctx().openUrl(url);
