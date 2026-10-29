# Changelog

What was built, when and why. It's written for whoever picks the work up next, people or agents: start with **For agents** below, then read the newest release. Newest first.

**Keeping it up to date:** add a line under **Unreleased** with each change you push (what changed for the user, and anything the next person needs to know). When CI publishes a release, move those lines under a heading for that version. The release notes are the **Unreleased** section, so write it for the user.

---

## For agents

### What this is

A desktop app (Windows first, Linux kept portable; Tauri 2) that keeps an offline library of software projects: add a project by its link, and its code and latest release are downloaded, checked for updates on a schedule with every earlier version kept, shown with their readme and details, and the latest release can be launched. The owner ("DrFlGd", Jerred) asked for it on 2026-10-29 with Model Library, SCAD Workshop and Game Repo as the design cues; the plan and the decisions are in `docs/PLAN.md`.

### Where things are

| Path | What |
| --- | --- |
| `web/` | The front end (no build step). `app.js`: start-up and routing (`#/`, `#/repo/<id>?tab=`, `#/activity`, `#/settings`, `#/about`). `web/ui/`: `shell.js` (mounts the islands: the menu with its tag, language and server filters, the status bar with the download in hand, keys, pasting a link anywhere opens Add), `state.js` (shared state, prefs, theme, polling the jobs: every second while there's work, every 15 s otherwise), `library.js` (cards and list, search, sort), `repo.js` (a project's page: readme, code versions, releases, the details/follow/tags inspector, Launch and its menu), `dialogs.js` (Add, confirm), `activity.js`, `settings.js`, `about.js`, `actions.js` (launch, check, open folder), `icons.js`. `lib/format.js`: sizes and dates. `platform.js` / `platform-desktop.js`: the seam between the page and the backend. |
| `desktop/core/` | Rust, no GUI. `api.rs` (the command table `App::call`, the job queue and its worker thread, the schedule thread), `library.rs` (the library folder, `repo.json`'s shape, scanning), `sources/` (`mod.rs`: reading links, the `Source` trait, ids; `github.rs`, `gitlab.rs`, `gitea.rs`), `capture.rs` (a check: details, a new snapshot of the code, a new release, each into its own folder through `.partial/`), `http.rs` (ureq: JSON, downloads with progress and Stop, no proxy for this computer or NO_PROXY), `archive.rs` (unpacking ZIP and .tar.gz safely), `readme.rs` (Markdown with ammonia; relative pictures from the captured files, relative links to the site), `launch.rs` (which files are for which system, launch candidates and their order, starting them), `config.rs` (this computer: the open library, tokens, prefs), `util.rs`. `src/bin/reposync-cli.rs`: `list`, `add`, `check-all`, `serve` (the backend over HTTP for the tests). |
| `desktop/src-tauri/` | The Tauri app: a thin layer over `api.rs`, plus the folder dialog, the file manager, web links in the browser and the `library://` protocol (pictures in readmes). Version in `tauri.conf.json`. Icons from `icons/icon.svg` (`npx @tauri-apps/cli@2 icon icons/icon.svg -o icons`, then delete the mobile ones). |
| `tools/` | `build_desktop.py` (web/ to `build/desktop/ui`, fonts with `--fetch-fonts`, `THIRD_PARTY.md` and `build-info.json` for About), `set_version.py`, `vendor_preact.py`. |
| `tests/` | `desktop_page.py` (acceptance test: the page + `reposync-cli serve` + `forge_mock.py`, in Chromium), `forge_mock.py` (GitHub's and GitLab's APIs over local git repositories, releases as folders), `tauri_shim.js`. |
| `.github/workflows/desktop.yml` | Windows app, tests and release; Linux only from Run workflow. |

### Conventions

- **Releases.** The repository keeps `0.<phase>.0` in `desktop/src-tauri/tauri.conf.json` and `desktop/Cargo.toml`. CI picks the next free `0.<phase>.<n>` from the release tags and publishes `v<version>` from `main` only when the Rust tests, the acceptance test and the Windows build pass.
- **Windows first**, as in the owner's other apps; keep the code portable.
- **Plan first** for a big feature: the design goes into `docs/PLAN.md` before the code.
- **Text:** plain words in the interface, no jargon; British spellings (licence, colour), as in the other apps.
- **The library folder is the source of truth** and must make sense without the app. No database in it; unknown keys in `repo.json` are kept (`#[serde(flatten)] extra`); JSON is written with sorted keys (`util::write_json`). Nothing the app captured is deleted except by the user (Delete version, Remove project).
- **Tests grow with each feature:** add checks to `tests/desktop_page.py` for everything the user can see, and to `forge_mock.py` for anything new asked of a server.

### Working in a cloud session

- crates.io and npm were reachable on 2026-10-29 (`cargo test -p reposync-core` works; `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev` let `cargo check -p repo-sync` build the Tauri app too). api.github.com and codeload.github.com were refused by the session's proxy, gitlab.com was reachable: `reposync-cli add https://gitlab.com/gitlab-org/cli --library /tmp/lib` is a good live check.
- **Acceptance test locally:** `python3 tools/build_desktop.py --out build/desktop`, then `python3 tests/desktop_page.py --cli desktop/target/debug/reposync-cli --ui build/desktop/ui --home /tmp/rs-home --out /tmp/rs-shots --chromium /opt/pw-browsers/chromium` (`pip install playwright`; the pre-installed Chromium works).
- `REPOSYNC_API="github.com=http://127.0.0.1:8795;gitlab.com=http://127.0.0.1:8795/api/v4"` points the sources at `forge_mock.py`; `REPOSYNC_CONFIG` gives `reposync-cli` another config folder.

### Known limitations now

- Checks run while the app is open (or from `reposync-cli check-all`); there's no tray icon or service yet.
- Only the latest release is captured on each check, not the history before the project was added.
- A plain git address (no GitHub, GitLab or Gitea API) can't be added yet; nor can private projects without a token.
- Launch starts installers as they are (each Launch runs the installer again); it can't tell which program an installer put on the computer.
- Downloads don't resume: a broken one is tried again whole at the next check.
- Tokens are stored in plain text in the app's config folder (not the system's keyring).
- Installers aren't code-signed, and the app doesn't update itself.

### Next (ideas, not yet chosen by the owner)

Keep running in the tray so checks happen with the window closed; capture several past releases when a project is added; full-text search across the captured code, not only readmes; a plain-git source; per-project "keep at most N versions"; the app updating itself from its own releases (as Game Repo does).

---

## Unreleased

First version of Repo Sync:

- **Add projects by their links**, one or many at once (paste a link anywhere in the window, or press N): GitHub, GitLab, Codeberg and other Gitea or Forgejo servers. Each link is read as you type, and a project already in the library is pointed out.
- **The code and the latest release are downloaded** into the library folder, each into its own folder, with the release's notes beside its files. Release files can be every file, only the ones for this computer, or only the notes, and a size limit can be set.
- **Scheduled checks keep every version.** Every project is checked every day by default (hourly to weekly, or by hand; per project too). A new commit or release is added beside the ones before; nothing is replaced. Activity shows the download in hand (with Stop), what's waiting and what each check found.
- **A page for each project**: its readme (pictures shown from the captured files, offline), size on disk, added, last captured, last checked, next check, updated upstream, latest commit and release, licence, language, stars and forks, topics; every code version (open its folder, see its readme, delete it) and every release (its files, notes, Launch, delete it).
- **Launch the latest release**: the best program for this computer is started (a portable ZIP is unpacked first); the Launch menu lists the others and remembers the choice.
- **Sort and search**: search names, descriptions, topics, tags and readmes; filter by your own tags, language or server; sort by update, capture, addition, name, size or stars; cards or a list.
- Settings: the library folder, the schedule, what new projects follow, the size limit, tokens for GitHub, GitLab and Codeberg, light or dark.
- `reposync-cli` (`list`, `add`, `check-all`) uses the same library without a window, for scheduled checks while the app is closed.
