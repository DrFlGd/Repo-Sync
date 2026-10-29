# Repo Sync: plan and decisions

## 1. The ask (2026-10-29)

Jerred: "review Model Library, SCAD Workshop and Game Library for design cues and build a software application that can act as a library for GitHub or other software repo/project sources. I want to be able to provide the app a GitHub link, download the source and the most recent release. I want the app to be able to regularly update source and releases based on repo changes (without deleting the previous versions). It should have an information view for each repo that captures its readme and information about size/date captured/date last updated etc. The GUI should be intuitive and streamlined for this task. I should be able to launch the latest release from the application."

"Game library" was read as [Game Repo](https://github.com/DrFlGd/Game-Repo), the closest name among the owner's repositories.

## 2. Design cues taken

| From | Taken |
| --- | --- |
| All three | Rust core with one command table (`App::call`), a thin Tauri 2 shell, a Preact + htm page with no build step, `platform.js` as the seam, `tests/tauri_shim.js` + `<cli> serve` for the acceptance test in Chromium, `CHANGELOG.md` with **For agents**, numbered releases from CI (`tools/set_version.py`), Windows first. |
| Model Library | The Drafting Sheet skin (square corners, fine rules, drafting blue, Overpass and Overpass Mono, corner ticks on cards, the status bar as a title block, mono section heads), the menu on the left with counts in brackets, the inspector on the right of a page, the library folder as the source of truth with sorted JSON and unknown keys kept. |
| SCAD Workshop | Adding GitHub projects by link without git (the API and a ZIP of one commit), update checks, a local stand-in for GitHub's API in the tests (`forge_mock.py` grows from its `github_mock.py`), tokens in Settings for the rate limit. |
| Game Repo | Jobs with progress and Stop in the status bar, the Activity view, releases with installers. |

## 3. The library folder

```
<library>/_library/library.json    { format: 1, id, name, created, settings }
<library>/<host>/<path>/repo.json  the project (below)
<library>/<host>/<path>/source/<YYYY-MM-DD_HHMM>_<sha7>/   one captured commit, unpacked
<library>/<host>/<path>/releases/<tag>/                      one captured release: files + RELEASE_NOTES.md
<library>/<host>/<path>/.partial/                            a download in progress (removed before each check)
```

`<path>` is "owner/name" (GitLab: "group/sub/name"), so a project's id is "github.com/owner/name" and is also its folder. `repo.json` holds `source` (kind, host, path), `details` (from the server: description, default branch, stars, forks, licence, language, topics, archived, pushed_at, created_at), `follow` (code, branch, releases, pre-releases, which files, hours between checks), the user's `tags` and `notes`, `last_checked`, `last_changed`, `last_error`, `snapshots` and `releases` (oldest first, each with its folder, dates, size) and `launch` (the file Launch starts, when the user chose one).

**Nothing is replaced.** A new commit on the followed branch becomes a new snapshot folder; a new latest release a new release folder. The same tag published again with a different date (a moving "nightly") gets `<tag>_<date>`. A download goes into `.partial/` and is moved into place only when complete, so a crash or Stop never leaves half a version among the kept ones. Versions are deleted only by the user (Delete version, Remove project).

## 4. Sources

`sources::Source` answers five questions: details, the newest commit on a branch, where to get a commit as a ZIP, the latest release, and where to get a release file. Kinds now: GitHub (and Enterprise at `/api/v3`), GitLab (API v4; groups; "upcoming" releases count as pre-releases), Gitea and Forgejo (API v1; Codeberg). Links are read by `parse_url` (web links with extra parts, `git@host:path`, "owner/name"); a host it doesn't know is asked as Gitea, then as GitLab. Tokens are per host, kept on this computer. A new kind is one more file in `sources/` and a line in `kind_of_host`.

## 5. Updates

The worker thread runs one job at a time (Add, Check) from a queue; a job for a project already queued or running isn't queued twice. The schedule thread looks every 30 s for projects due a check: `last_checked` + the project's hours (or the library's, 24 by default; 0 is never). A failed check records its error and counts as a check, so a broken project isn't retried in a loop. Checks run while the app is open; `reposync-cli check-all` does the same from a scheduled task.

## 6. The interface

- **Library**: cards (name, server/owner, description, latest release, language, stars, size, versions kept, updated upstream, Launch) or a list; search (names, descriptions, topics, tags, readme text, in the backend); the menu filters by tag, language, server and "Check failed"; sort by updated upstream, captured, added, name, size, stars. Projects being added show as dashed cards.
- **A project's page**: Launch (split button: the menu lists what can be started and remembers the choice), Check now, Folder, the site, ⋯ (copy link, remove). Tabs: Readme (of the latest version, or an earlier one from Code versions), Code versions, Releases (files kept or skipped and why, notes, Launch, folder, delete). Inspector: details, topics, tags, Follow.
- **Activity**: the job in hand with progress and Stop, the queue, what each finished job found.
- **Settings**: library folder, schedule, size limit, what new projects follow, tokens, theme.
- **Add**: a box for one link or many, read as typed; what to follow.

## 7. Launch

`launch::candidates` lists the release folder's files that run on this computer: programs (`.exe`, `.AppImage`, executables, `.jar`), installers (`setup*.exe`, `.msi`, `.deb`, `.dmg`…), scripts (`.cmd`, `.sh`…) and archives (`.zip`, `.tar.gz`), skipping files named for another system or processor. Programs come first, then portable archives, installers, scripts; a file named after the project and one named for this system rank higher; uninstallers, helpers and redistributables lower. An archive is unpacked once into a folder beside it and its best program started.

## 8. Phase 1 notes (0.1, 2026-10-29)

Built as above. Checked in a cloud session: the Rust tests, the acceptance test (GitHub and GitLab through `forge_mock.py`: adding, the readme, details, Launch, Check now, the schedule, following, tags, deleting, removing, changing library), and a live add of gitlab.com/gitlab-org/cli (25 MB of code, its latest release filtered to this computer). GitHub itself couldn't be reached from the session; the GitHub source follows the documented API and SCAD Workshop's working code.

## 9. After testing 0.1 (2026-10-29)

Jerred tried 0.1.0 on terrain-press and asked for: a releases view with a box per release to choose which are kept (besides the newest, which checks still take); the Follow options as lists filled from the project (the branch was the one typed field); and adding a GitHub user to capture all of their projects. Decisions:

- **Releases tab.** Lists what the server has (`releases_available`, up to 300), newest first, merged with what's kept. Ticking queues a `release` job for that tag (it sits beside a check of the same project in the queue); clearing a kept one deletes its folder after a confirm, as Delete does. `repo.json`'s `releases` are kept in publish order, so "the last one" stays the newest for Launch and the details.
- **Accounts.** A link with only an owner (`github.com/someone`, `github.com/orgs/x`, `gitlab.com/groups/x`, `codeberg.org/someone`) is an account. Adding one queues an `account` job: it lists the projects, queues an Add for each one not in the library (forks only when asked) and, unless unticked, follows the account in `_library/accounts.json` (in the library, so it travels with it). The schedule looks at followed accounts at the library's interval; Check all looks too. A project of a followed account removed by hand is remembered in `removed` and not added back.
- **Wording.** Tags are "Your tags" with a line saying they aren't git tags; "Only files for <system>" with a line saying what's skipped.
