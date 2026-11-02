// Collections: projects grouped under names of the user's own, kept in the
// library folder (_library/collections.json). Making, renaming and deleting them,
// and putting projects in and out.
import { ui, api, toast, errText, loadRepos } from "./state.js";
import { promptDialog, confirmDialog, chooseDialog } from "./dialogs.js";

const nameTaken = (name, except = null) => {
  const n = name.toLowerCase();
  if (ui.get().collections.some((c) => c.id !== except && c.name.toLowerCase() === n)) throw new Error(`There's a collection called ${name} already.`);
};

/** Make a collection, with `ids` in it to start. Resolves the new collection, or null. */
export async function newCollection(ids = []) {
  let made = null;
  await promptDialog({
    title: "New collection", label: "Name", ok: "Make the collection", placeholder: "Like Game tools, or To try",
    check: async (name) => { nameTaken(name); made = await api("collection_create", { name, ids }); },
  });
  if (!made) return null;
  await loadRepos();
  toast(ids.length ? `Made ${made.name}, with ${ids.length === 1 ? "this project" : `${ids.length} projects`}` : `Made ${made.name}`);
  return made;
}

export async function renameCollection(c) {
  const name = await promptDialog({
    title: `Rename ${c.name}`, label: "New name", value: c.name, ok: "Rename",
    check: async (name) => { nameTaken(name, c.id); await api("collection_rename", { collection: c.id, name }); },
  });
  if (name) await loadRepos();
}

export async function deleteCollection(c) {
  const n = c.repos.length;
  if (!(await confirmDialog({
    title: `Delete the collection ${c.name}?`,
    body: `Only the collection goes: its ${n === 1 ? "project stays" : `${n} projects stay`} in the library, with every version kept.`,
    ok: "Delete the collection",
  }))) return false;
  try {
    await api("collection_delete", { collection: c.id });
    const f = ui.get().filter;
    if (f?.by === "collection" && f.value === c.id) ui.set({ filter: null });
    await loadRepos();
    toast(`Deleted the collection ${c.name}`);
    return true;
  } catch (e) { toast(errText(e), 6000); return false; }
}

/** Put a project in a collection, or take it out. */
export async function setInCollection(c, id, on) {
  try {
    await api(on ? "collection_add" : "collection_remove", { collection: c.id, ids: [id] });
  } catch (e) { toast(errText(e), 6000); }
  await loadRepos();
}

/** Tick the projects in a collection from the whole library. */
export async function chooseProjects(c) {
  const r = await chooseDialog({ title: `Projects in ${c.name}`, chosen: c.repos });
  if (!r) return;
  try {
    if (r.add.length) await api("collection_add", { collection: c.id, ids: r.add });
    if (r.remove.length) await api("collection_remove", { collection: c.id, ids: r.remove });
  } catch (e) { toast(errText(e), 6000); }
  await loadRepos();
}
