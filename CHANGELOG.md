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
| `desktop/core/` | Rust, no GUI. `api.rs` (the command table `App::call`, the job queue and its worker thread, the schedule thread), `library.rs` (the library folder, `repo.json`'s shape, scanning), `sources/` (`mod.rs`: reading links, the `Source` trait, ids; `github.rs`, `gitlab.rs`, `gitea.rs`; `account.rs`: links to users and organisations, listing their projects), `capture.rs` (a check: details, a new snapshot of the code, a new release, each into its own folder through `.partial/`), `http.rs` (ureq: JSON, downloads with progress and Stop, no proxy for this computer or NO_PROXY), `archive.rs` (unpacking ZIP and .tar.gz safely), `readme.rs` (Markdown with ammonia; relative pictures from the captured files, relative links to the site), `launch.rs` (which files are for which system, launch candidates and their order, starting them), `config.rs` (this computer: the open library, tokens, prefs), `util.rs`. `src/bin/reposync-cli.rs`: `list`, `add`, `check-all`, `serve` (the backend over HTTP for the tests). |
| `desktop/src-tauri/` | The Tauri app: a thin layer over `api.rs`, plus the folder dialog, the file manager, web links in the browser and the `library://` protocol (pictures in readmes). Version in `tauri.conf.json`. Icons from `icons/icon.svg` (`npx @tauri-apps/cli@2 icon icons/icon.svg -o icons`, then delete the mobile ones). |
| `tools/` | `build_desktop.py` (web/ to `build/desktop/ui`, fonts with `--fetch-fonts`, `THIRD_PARTY.md` and `build-info.json` for About), `set_version.py`, `vendor_preact.py`. |
| `tests/` | `desktop_page.py` (acceptance test: the page + `reposync-cli serve` + `forge_mock.py`, in Chromium; its websites section opens hosted sites with the internet blocked), `forge_mock.py` (GitHub's and GitLab's APIs over local git repositories, releases as folders, `/pages`), `cdn_mock.py` (jsDelivr, Google Fonts and a live website), `tauri_shim.js`. |
| `desktop/core/src/sites/` | Hosting websites (docs/PLAN.md section 10): `detect.rs`, `router.rs`, `server.rs`, `vendor.rs`, `rewrite.rs`, `mirror.rs`, `build.rs`, `export.rs`, `scan.rs`, `mime.rs`; the commands are in `api_site.rs`; the page's side is `web/ui/site.js` (Website tab, Websites page). |
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
- `REPOSYNC_API="github.com=http://127.0.0.1:8795;gitlab.com=http://127.0.0.1:8795/api/v4"` points the sources at `forge_mock.py`; `REPOSYNC_VENDOR="cdn.jsdelivr.net=http://127.0.0.1:8796/jsdelivr;…"` points CDNs at `cdn_mock.py`; `REPOSYNC_CONFIG` gives `reposync-cli` another config folder.
- apt was refused by the session's proxy on 2026-11-11, so the Tauri crate couldn't be checked locally then; CI's Windows build checks it.

### Known limitations now

- Checks run while the app is open (or from `reposync-cli check-all`); there's no tray icon or service yet.
- Each check captures only the newest release; earlier ones are kept when ticked in the Releases tab (the app lists at most 300 a project).
- A followed account lists only public projects on GitHub (private ones need adding one by one).
- A plain git address (no GitHub, GitLab or Gitea API) can't be added yet; nor can private projects without a token.
- Launch starts installers as they are (each Launch runs the installer again); it can't tell which program an installer put on the computer.
- A lowered limit to keep takes effect at the project's next check, not at once.
- Downloads don't resume: a broken one is tried again whole at the next check.
- Tokens are stored in plain text in the app's config folder (not the system's keyring).
- Hosted sites are served while the app (or `reposync-cli host`) is open. A site's own API calls, and addresses a script builds while it runs on a CDN that wasn't seen, still need the internet. Service workers are served as they are.
- Installers aren't code-signed, and the app doesn't update itself.

### Next (ideas, not yet chosen by the owner)

Keep running in the tray so checks happen with the window closed; full-text search across the captured code, not only readmes; a plain-git source; per-project "keep at most N versions"; the app updating itself from its own releases (as Game Repo does).

---

## Unreleased

**Hosting websites (0.2).** Jerred asked for built-in hosting of repos that would normally use GitHub Pages, as general as possible, with Agilancer/vectorclad as the example:

- **Open a project's website from the library.** A project that publishes a site (a GitHub Pages workflow, a `gh-pages` branch, a `docs/` folder, GitLab Pages…) gets a Website tab and an **Open site** button: the site opens in your browser at its own address on this computer, such as `http://vectorclad.agilancer.localhost:<port>/vectorclad/`, served as GitHub Pages serves it (folders, `404.html`, clean addresses, `_redirects`). Each version has an address of its own too, so you can open last month's.
- **Works offline.** After each capture, the scripts, styles and fonts the site loads from CDNs (jsDelivr, unpkg, esm.sh, cdnjs, Google Fonts…) are saved into the library, each file kept once, and the site uses the saved copies. The Website tab says what was saved, which servers it still needs the internet for, and lists its recent requests. "Block the internet" tests that it really works offline.
- **A site's published branch is captured too** (`gh-pages`), so a site built by a workflow can be opened as it was published.
- **Save any website as a project.** Add a web address (or tick "Save these as websites") and its pages are saved as they're served, with the files they load, checked on the schedule like code; a capture that's the same as the last isn't kept. A repo's live site can be captured too (Capture live site, or at each check). Compare a capture with the one before, page by page. Pages a capture missed are saved as you browse.
- **Build a site on this computer** (Vite, Next, Hugo, MkDocs, Jekyll…), only after you agree to run the project's code; the build runs in a copy and the captured version isn't changed.
- **Export a site as a repo of its own**: a folder, a ZIP or a Git folder, with its CDN files inside and its addresses pointed at them, a README saying where it came from, and optionally a GitHub Pages workflow; or push it to an empty GitHub repo you've made.
- The **Websites** page lists every hosted site; the status bar shows the server's port. The library's search finds words on saved websites' pages.
- `reposync-cli host` hosts the library's sites without the window; `mirror`, `site` and `export` do the rest from the command line.

For agents (0.2): the design is docs/PLAN.md section 10. New `desktop/core/src/sites/` (`detect` what a version holds, `router` Pages' rules, `server` tiny_http on 127.0.0.1 and ::1 with one origin per site, `vendor` CDN files into `_library/sites/blobs` with a lock per version, `rewrite` addresses as served, `mirror` captures of websites, `build` local builds, `export`, `scan` reading HTML/JS/CSS, `mime`) and `api_site.rs` (the `site_*` commands, the `site` job, `add_website`). `repo.json` gained `pages`, `live` (versions like `snapshots`), `site` (settings), `snapshots[].site` (what was detected), `follow.site_branch`/`live`, `details.has_pages`/`pages`; website projects are `source.kind: "web"`, id `web/<domain>/<page>`. `REPOSYNC_VENDOR` points CDN hosts at `tests/cdn_mock.py`; the site server's port is `site_port` in this computer's config. The main window no longer counts `*.localhost` as its own (sites open in the browser).

After testing 0.1.0 on terrain-press, Jerred asked for these:

- **Add every project of a GitHub user or organisation** (or a GitLab user or group, or a Codeberg/Gitea user): paste a link such as `https://github.com/someone`. The account is followed, so its new projects are added at each scheduled check (or Check all); its forks are left out unless asked, and a project removed by hand isn't added back. Settings lists the followed accounts, with Stop following.
- **Pick which releases to keep.** The Releases tab lists every release on the server (pre-releases can be hidden), with a box to keep each one: ticking downloads it, clearing a tick deletes its files. The newest release is still downloaded by each check. Releases are kept in the order they were published, so an older one picked later never becomes the one Launch starts.
- **Keep only the newest versions, if you like.** Settings has separate limits for versions of the code and for releases ("Every one", "Only the newest", the newest 2, 3, 5 or 10), and a project can have its own in Follow. After each check, the oldest past the limit are deleted; releases ticked by hand are always kept (marked "picked"). Activity says what was deleted.
- **The branch to follow is picked from a list** of the project's branches (typed by hand only when the server can't list them).
- Clearer words: "Your tags" says they're your own labels, not the project's git tags; "Only files for Windows" (was "for this computer") says which files are skipped; "The newest release" (was "Releases") in Follow.
- Changes to a project's follow settings or tags made in quick succession are saved in the order made.

For agents: `Source` gained `releases(limit)` and `branches()` (paged; the mock serves both); `sources/account.rs` reads account links and lists an account's projects; followed accounts live in `_library/accounts.json`; `capture::prune` applies the limits to keep (`Settings.keep_source`/`keep_releases`, `Follow.keep_source`/`keep_releases`, 0 or null for every one; `ReleaseRecord.picked` exempts hand-picked releases); new jobs `account` and `release`, new commands `releases_available`, `release_get`, `branches`, `accounts`, `account_remove`.

## 0.1.0 (2026-10-29)

First version of Repo Sync:

- **Add projects by their links**, one or many at once (paste a link anywhere in the window, or press N): GitHub, GitLab, Codeberg and other Gitea or Forgejo servers. Each link is read as you type, and a project already in the library is pointed out.
- **The code and the latest release are downloaded** into the library folder, each into its own folder, with the release's notes beside its files. Release files can be every file, only the ones for this computer, or only the notes, and a size limit can be set.
- **Scheduled checks keep every version.** Every project is checked every day by default (hourly to weekly, or by hand; per project too). A new commit or release is added beside the ones before; nothing is replaced. Activity shows the download in hand (with Stop), what's waiting and what each check found.
- **A page for each project**: its readme (pictures shown from the captured files, offline), size on disk, added, last captured, last checked, next check, updated upstream, latest commit and release, licence, language, stars and forks, topics; every code version (open its folder, see its readme, delete it) and every release (its files, notes, Launch, delete it).
- **Launch the latest release**: the best program for this computer is started (a portable ZIP is unpacked first); the Launch menu lists the others and remembers the choice.
- **Sort and search**: search names, descriptions, topics, tags and readmes; filter by your own tags, language or server; sort by update, capture, addition, name, size or stars; cards or a list.
- Settings: the library folder, the schedule, what new projects follow, the size limit, tokens for GitHub, GitLab and Codeberg, light or dark.
- `reposync-cli` (`list`, `add`, `check-all`) uses the same library without a window, for scheduled checks while the app is closed.
