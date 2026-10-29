#!/usr/bin/env python3
"""Acceptance test: the page against the app's real backend, in Chromium.

    python3 tests/desktop_page.py --cli desktop/target/release/reposync-cli \
        --ui build/desktop/ui --home /tmp/rs-home --out shots [--chromium /opt/pw-browsers/chromium]

Makes test projects (local git repositories with releases), serves them with
tests/forge_mock.py as GitHub and GitLab, runs `reposync-cli serve` with
tests/tauri_shim.js standing in for Tauri, and drives the page: adding projects,
the readme and details, launching the latest release, capturing changes upstream
by hand and on the schedule (keeping every earlier version), following settings,
tags, search, removing, and changing the library folder. Screenshots go to --out.
"""
import argparse
import json
import os
import shutil
import socket
import struct
import subprocess
import sys
import time
import urllib.request
import zlib
from pathlib import Path

from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parent.parent
ap = argparse.ArgumentParser()
ap.add_argument("--cli", required=True)
ap.add_argument("--ui", required=True)
ap.add_argument("--home", type=Path, required=True)
ap.add_argument("--out", type=Path, required=True)
ap.add_argument("--chromium", default=None)
a = ap.parse_args()

WINDOWS = os.name == "nt"
home = a.home.resolve()
if home.exists():
    shutil.rmtree(home)
repos = home / "upstream"
lib = home / "Library"
repos.mkdir(parents=True)
a.out.mkdir(parents=True, exist_ok=True)
failures = []


def check(cond, what):
    print(("ok   " if cond else "FAIL ") + what, flush=True)
    if not cond:
        failures.append(what)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def png(w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))
    chunk = lambda t, d: struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def git(repo, *args):
    env = {**os.environ, "GIT_AUTHOR_NAME": "Test", "GIT_AUTHOR_EMAIL": "t@example.com", "GIT_COMMITTER_NAME": "Test", "GIT_COMMITTER_EMAIL": "t@example.com"}
    return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True, env=env).stdout


LAUNCHER_SH = "echo launched > launched.txt\n"
LAUNCHER_CMD = "@echo launched> launched.txt\r\n"


def make_project(owner, name, readme):
    r = repos / f"{owner}__{name}"
    r.mkdir()
    git(r, "init", "-q", "-b", "main")
    (r / "README.md").write_text(readme, encoding="utf-8")
    (r / "docs").mkdir()
    (r / "docs" / "shot.png").write_bytes(png(40, 24, (28, 90, 166)))
    (r / "src.txt").write_text("version 1\n")
    git(r, "add", "-A")
    git(r, "commit", "-qm", "First version")
    return r


def make_release(owner, name, tag, published, extra=None):
    d = repos / f"{owner}__{name}.releases" / tag
    d.mkdir(parents=True)
    (d / f"{name}-linux.sh").write_bytes(LAUNCHER_SH.encode())
    (d / f"{name}-windows.cmd").write_bytes(LAUNCHER_CMD.encode())
    (d / f"{name}-macos.dmg").write_bytes(b"dmg")
    (d / "SHA256SUMS.txt").write_text("sums\n")
    (d / "release.json").write_text(json.dumps({"name": f"{name} {tag}", "body": f"## What's new in {tag}\n\n- Faster.\n", "published_at": published, **(extra or {})}))


def new_commit(repo, text):
    (repo / "src.txt").write_text(text)
    git(repo, "commit", "-qam", text.strip())


tool = make_project("demo", "tool", "# Demo Tool\n\n![shot](docs/shot.png)\n\nA small tool. The word zebrafish is only in this readme.\n\n<script>window.__pwned = 1</script>\n\n[Guide](docs/GUIDE.md)\n")
make_release("demo", "tool", "v1.0", "2026-01-01T10:00:00Z")
lab = make_project("demo", "lab", "# Lab Project\n\nOn GitLab.\n")
make_release("demo", "lab", "v0.9", "2026-01-02T10:00:00Z")
git(lab, "branch", "dev")

mock_port, port = free_port(), free_port()
mock = subprocess.Popen([sys.executable, str(ROOT / "tests/forge_mock.py"), "--repos", str(repos), "--port", str(mock_port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
env = {**os.environ, "REPOSYNC_API": f"github.com=http://127.0.0.1:{mock_port};gitlab.com=http://127.0.0.1:{mock_port}/api/v4"}
serve_log = open(a.out / "serve.log", "w")
serve = subprocess.Popen([a.cli, "serve", "--ui", a.ui, "--home", str(home / "app"), "--library", str(lib), "--port", str(port), "--tick", "1"],
                         env=env, stdout=serve_log, stderr=subprocess.STDOUT)
BASE = f"http://127.0.0.1:{port}"
for _ in range(100):
    try:
        urllib.request.urlopen(BASE + "/", timeout=1)
        break
    except Exception:
        time.sleep(0.1)


def wait_for(fn, what, timeout=30):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if fn():
                check(True, what)
                return True
        except Exception:
            pass
        time.sleep(0.25)
    check(False, what + " (timed out)")
    return False


def repo_json(id_):
    return json.loads((lib / id_ / "repo.json").read_text(encoding="utf-8"))


try:
    with sync_playwright() as p:
        browser = p.chromium.launch(executable_path=a.chromium) if a.chromium else p.chromium.launch()
        page = browser.new_page(viewport={"width": 1400, "height": 880})
        page.add_init_script((ROOT / "tests/tauri_shim.js").read_text())
        page.goto(BASE + "/")
        page.wait_for_selector(".empty-state")
        check(page.is_visible("text=Add your first project"), "an empty library offers to add a project")
        page.screenshot(path=str(a.out / "01-empty.png"))

        # ---- adding projects -------------------------------------------------
        page.fill("#first-link", "https://github.com/demo/tool")
        page.click(".empty-state button[type=submit]")
        page.wait_for_selector("#add-links")
        page.fill("#add-links", "https://github.com/demo/tool\nhttps://gitlab.com/demo/lab/-/releases\nnot a link\nhttps://github.com/demo/missing")
        page.wait_for_selector(".checks li >> nth=3")
        checks = page.inner_text(".checks")
        check("github.com/demo/tool" in checks and "gitlab.com/demo/lab" in checks, "the Add dialog reads each link")
        check("doesn't look like a link" in checks, "a line that isn't a link is pointed out")
        page.screenshot(path=str(a.out / "02-add.png"))
        check(page.inner_text("#add-go").strip().endswith("Add 3 projects"), "three links can be added")
        page.click("#add-go")
        wait_for(lambda: page.locator(".card:not(.pending)").count() == 2 and page.locator(".card .badge.working").count() == 0, "both projects are added and captured", 40)
        wait_for(lambda: page.evaluate("window.__reposync.ui.get().jobs.current === null"), "the jobs finish")
        page.wait_for_timeout(500)
        check(page.locator(".card").count() == 2, "the project that doesn't exist isn't added")
        check((lib / "github.com/demo/tool/repo.json").is_file() and (lib / "gitlab.com/demo/lab/repo.json").is_file(), "each project has a folder with repo.json")
        check(not (lib / "github.com/demo/missing").exists(), "a failed add leaves no folder behind")
        rj = repo_json("github.com/demo/tool")
        check(len(rj["snapshots"]) == 1 and len(rj["releases"]) == 1, "the code and the latest release were captured")
        snap1 = rj["snapshots"][0]["folder"]
        check((lib / "github.com/demo/tool/source" / snap1 / "src.txt").read_text() == "version 1\n", "the code is unpacked into source/<version>/")
        check((lib / "github.com/demo/tool/releases/v1.0/tool-linux.sh").is_file(), "the release's files are in releases/<tag>/")
        check((lib / "github.com/demo/tool/releases/v1.0/RELEASE_NOTES.md").is_file(), "the release's notes are kept beside its files")
        card = page.locator(".card[data-id='github.com/demo/tool']")
        check("v1.0" in card.inner_text(), "the card shows the latest release")
        page.click("text=Activity")
        page.wait_for_selector("#finished")
        check("doesn't exist" in page.inner_text("#finished"), "Activity says why a project couldn't be added")
        page.screenshot(path=str(a.out / "03-activity.png"))
        page.click(".sidebar >> text=Library")
        page.wait_for_selector(".card")
        page.screenshot(path=str(a.out / "04-library.png"))

        # ---- search, the list -----------------------------------------------
        page.fill("#search", "zebrafish")
        wait_for(lambda: page.locator(".card").count() == 1, "search finds words in readmes")
        check("Found in the readme" in page.inner_text(".card"), "a readme match is labelled")
        page.fill("#search", "")
        wait_for(lambda: page.locator(".card").count() == 2, "clearing the search shows everything")
        page.click("button[title=List]")
        page.wait_for_selector("table.list tr.row")
        check(page.locator("table.list tr.row").count() == 2, "the list view has a row per project")
        page.click("button[title=Cards]")

        # ---- a project's page --------------------------------------------------
        page.click(".card[data-id='github.com/demo/tool']")
        page.wait_for_selector("#readme h1")
        check(page.inner_text("#readme h1") == "Demo Tool", "the readme is shown")
        img = page.locator("#readme img")
        check("/library/github.com/demo/tool/source/" in (img.get_attribute("src") or ""), "pictures in the readme come from the captured files")
        wait_for(lambda: page.evaluate("document.querySelector('#readme img').naturalWidth") == 40, "the readme's picture loads offline")
        check(page.evaluate("window.__pwned === undefined") and page.locator("#readme script").count() == 0, "scripts in a readme don't run")
        check("github.com/demo/tool/blob/" in (page.get_attribute("#readme a", "href") or ""), "relative links in a readme lead to the project's site")
        props = page.inner_text("#props")
        for label in ("Size on disk", "Added", "Last captured", "Last checked", "Updated upstream", "Latest release", "MIT"):
            check(label in props, f"details show {label}")
        page.screenshot(path=str(a.out / "05-project.png"))

        # ---- launching the latest release --------------------------------------
        page.wait_for_selector("#launch:not([disabled])")
        launched = lib / "github.com/demo/tool/releases/v1.0/launched.txt"
        page.click("#launch")
        wait_for(launched.is_file, "Launch starts the latest release's program")
        page.click("#launch-menu")
        menu = page.inner_text(".menu")
        mine, other = ("tool-windows.cmd", "tool-linux.sh") if WINDOWS else ("tool-linux.sh", "tool-windows.cmd")
        check(mine in menu and other not in menu and "dmg" not in menu, "Launch offers only what runs on this computer")
        page.keyboard.press("Escape")

        # ---- a change upstream, captured by hand --------------------------------
        new_commit(tool, "version 2\n")
        make_release("demo", "tool", "v1.1", "2026-02-01T10:00:00Z")
        page.click("#check-now")
        wait_for(lambda: len(repo_json("github.com/demo/tool")["snapshots"]) == 2 and len(repo_json("github.com/demo/tool")["releases"]) == 2, "Check now captures the new version and release")
        rj = repo_json("github.com/demo/tool")
        check((lib / "github.com/demo/tool/source" / snap1 / "src.txt").read_text() == "version 1\n", "the earlier version of the code is kept")
        check((lib / "github.com/demo/tool/releases/v1.0/tool-linux.sh").is_file() and (lib / "github.com/demo/tool/releases/v1.1/tool-linux.sh").is_file(), "the earlier release is kept beside the new one")
        page.wait_for_selector("#tab-versions .n:text('2')")
        page.click("#tab-versions")
        wait_for(lambda: page.locator("#versions tbody tr").count() == 2, "Code versions lists both versions")
        page.screenshot(path=str(a.out / "06-versions.png"))
        page.click("#tab-releases")
        wait_for(lambda: page.locator("#releases .release").count() == 2, "Releases lists both releases")
        page.click("#releases .release >> nth=0 >> text=Show the notes")
        page.wait_for_selector("#releases .notes h2")
        check("What's new in v1.1" in page.inner_text("#releases .notes"), "a release's notes are shown")
        page.screenshot(path=str(a.out / "07-releases.png"))
        check("v1.1" in page.inner_text("#launch"), "Launch now starts the newer release")
        page.click("#launch")
        wait_for((lib / "github.com/demo/tool/releases/v1.1/launched.txt").is_file, "the newer release is launched")

        # ---- checking again with nothing new -------------------------------------
        page.click("#check-now")
        wait_for(lambda: page.evaluate("window.__reposync.ui.get().jobs.recent[0]?.message") == "Up to date", "a check with nothing new says it's up to date")
        check(len(repo_json("github.com/demo/tool")["snapshots"]) == 2, "nothing is captured twice")

        # ---- the schedule ---------------------------------------------------------
        new_commit(tool, "version 3\n")
        page.evaluate("window.__reposync.ctx().api('settings_set', { library: { check_every_hours: 0.0005, max_file_mb: 0, new_follow: { source: true, releases: true, prereleases: false, files: 'all', branch: '', every_hours: null } } })")
        wait_for(lambda: len(repo_json("github.com/demo/tool")["snapshots"]) == 3, "the schedule captures a change by itself", 40)
        page.evaluate("window.__reposync.ctx().api('settings_set', { library: { check_every_hours: 24, max_file_mb: 0, new_follow: { source: true, releases: true, prereleases: false, files: 'all', branch: '', every_hours: null } } })")
        wait_for(lambda: page.evaluate("window.__reposync.ui.get().jobs.current === null && window.__reposync.ui.get().jobs.queued.length === 0"), "scheduled checks finish")
        recent = page.evaluate("window.__reposync.ctx().api('jobs').then((j) => j.recent)")
        check(any(f["job"]["by"] == "schedule" for f in recent), "Activity records scheduled checks")

        # ---- picking releases ---------------------------------------------------------
        make_release("demo", "tool", "v0.9", "2025-12-01T10:00:00Z")
        make_release("demo", "tool", "v1.2-beta", "2026-02-15T10:00:00Z", {"prerelease": True})
        page.goto(BASE + "/#/repo/github.com/demo/tool?tab=releases")
        page.click("#available-refresh")
        wait_for(lambda: page.locator("#available tbody tr").count() == 4, "every release on the server is listed")
        row = lambda tag: page.locator(f"#available tr[data-tag='{tag}']")
        check(row("v1.1").locator("input").is_checked() and row("v1.0").locator("input").is_checked() and not row("v0.9").locator("input").is_checked(), "kept releases are ticked")
        check("pre-release" in row("v1.2-beta").inner_text(), "a pre-release is marked")
        page.screenshot(path=str(a.out / "07b-available.png"))
        row("v0.9").locator("input").check()
        wait_for(lambda: [r["tag"] for r in repo_json("github.com/demo/tool")["releases"]] == ["v0.9", "v1.0", "v1.1"], "ticking a release downloads it, in publish order")
        check((lib / "github.com/demo/tool/releases/v0.9/tool-linux.sh").is_file(), "the picked release's files are kept")
        wait_for(lambda: "kept" in row("v0.9").inner_text(), "the picked release shows as kept")
        check("v1.1" in page.inner_text("#launch"), "an older release picked later doesn't replace the newest for Launch")
        row("v0.9").locator("input").uncheck()
        page.click("#confirm-ok")
        wait_for(lambda: not (lib / "github.com/demo/tool/releases/v0.9").exists(), "clearing a tick deletes that release")
        wait_for(lambda: not row("v0.9").locator("input").is_checked(), "the cleared release is unticked")
        page.uncheck("#available-pre")
        wait_for(lambda: row("v1.2-beta").count() == 0, "pre-releases can be hidden")

        # ---- following, tags --------------------------------------------------------
        page.goto(BASE + "/#/repo/gitlab.com/demo/lab")
        page.wait_for_selector("#follow")
        page.select_option("#follow select[aria-label='Release files to keep']", "this-computer")
        page.select_option("#follow-every", "168")
        wait_for(lambda: repo_json("gitlab.com/demo/lab")["follow"]["files"] == "this-computer" and repo_json("gitlab.com/demo/lab")["follow"]["every_hours"] == 168, "follow settings are saved in repo.json")
        check("are skipped" in page.inner_text("#follow"), "the files choice says what's skipped")
        page.wait_for_selector("#follow-branch option[value='dev']", state="attached")
        check(page.locator("#follow-branch option[value='main']").count() == 0, "the branch list offers the project's branches beside the default")
        page.select_option("#follow-branch", "dev")
        wait_for(lambda: repo_json("gitlab.com/demo/lab")["follow"]["branch"] == "dev", "a branch can be picked from the list")
        page.select_option("#follow-branch", "")
        wait_for(lambda: repo_json("gitlab.com/demo/lab")["follow"]["branch"] == "", "the default branch can be picked again")
        check(page.inner_text("#props").count("GitLab") >= 1, "a GitLab project says where it's from")
        page.fill("#tag-input", "tools")
        page.press("#tag-input", "Enter")
        wait_for(lambda: repo_json("gitlab.com/demo/lab")["tags"] == ["tools"], "a tag is saved")
        make_release("demo", "lab", "v1.0", "2026-03-01T10:00:00Z")
        page.click("#check-now")
        wait_for(lambda: len(repo_json("gitlab.com/demo/lab")["releases"]) == 2, "the GitLab project's new release is captured")
        assets = {x["name"]: x for x in repo_json("gitlab.com/demo/lab")["releases"][-1]["assets"]}
        other_os = "lab-linux.sh" if WINDOWS else "lab-windows.cmd"
        check(assets[other_os]["file"] == "" and assets[other_os]["skipped"] == "for another system", "files for another system are skipped when asked")
        check(assets["SHA256SUMS.txt"]["file"] != "", "files for any system are kept")
        page.goto(BASE + "/#/")
        page.wait_for_selector(".filters")
        page.click(".filters >> text=tools")
        wait_for(lambda: page.locator(".card").count() == 1, "a tag in the menu narrows the library")
        page.screenshot(path=str(a.out / "08-tag.png"))
        page.click(".filters >> text=tools")

        # ---- deleting a version, removing a project -----------------------------------
        page.goto(BASE + "/#/repo/github.com/demo/tool?tab=versions")
        page.wait_for_selector("#versions tbody tr >> nth=2")
        page.click("#versions tbody tr >> nth=2 >> button[title='Delete this version']")
        page.click("#confirm-ok")
        wait_for(lambda: len(repo_json("github.com/demo/tool")["snapshots"]) == 2, "a version can be deleted")
        check(not (lib / "github.com/demo/tool/source" / snap1).exists(), "its folder is gone")
        page.goto(BASE + "/#/repo/gitlab.com/demo/lab")
        page.wait_for_selector("#repo-more")
        page.click("#repo-more")
        page.click("#remove")
        page.click("#confirm-ok")
        wait_for(lambda: not (lib / "gitlab.com").exists(), "removing a project deletes its folder")
        page.wait_for_selector(".card")
        check(page.locator(".card").count() == 1, "the removed project leaves the library")

        # ---- another library folder, and back --------------------------------------------
        other = home / "Other library"
        urllib.request.urlopen(urllib.request.Request(BASE + "/test/pick", data=json.dumps({"path": str(other)}).encode(), method="POST"))
        page.goto(BASE + "/#/settings")
        page.click("#change-library")
        wait_for(lambda: str(other) in page.inner_text("#library-path"), "the library folder can be changed")
        check((other / "_library/library.json").is_file(), "a new library folder is set up")
        page.goto(BASE + "/#/")
        page.wait_for_selector(".empty-state")
        urllib.request.urlopen(urllib.request.Request(BASE + "/test/pick", data=json.dumps({"path": str(lib)}).encode(), method="POST"))
        page.goto(BASE + "/#/settings")
        page.click("#change-library")
        page.goto(BASE + "/#/")
        wait_for(lambda: page.locator(".card").count() == 1, "the first library opens again with its project")

        # ---- every project of an account ---------------------------------------------
        for n in ("alpha", "beta", "old-fork"):
            make_project("crew", n, f"# {n}\n")
        api = lambda cmd, args: page.evaluate(f"window.__reposync.ctx().api('{cmd}', {json.dumps(args)})")
        idle = lambda: page.evaluate("window.__reposync.ctx().api('jobs').then((j) => j.current === null && j.queued.length === 0)")
        page.keyboard.press("n")
        page.wait_for_selector("#add-links")
        page.fill("#add-links", "https://github.com/crew")
        page.wait_for_selector(".checks li")
        check("every project of this GitHub account" in page.inner_text(".checks"), "a link to a user is read as an account")
        check(page.is_checked("#add-watch") and not page.is_checked("#add-forks"), "an account is followed, and its forks left out, unless asked")
        page.screenshot(path=str(a.out / "08b-account.png"))
        page.click("#add-go")
        wait_for(lambda: (lib / "github.com/crew/alpha/repo.json").is_file() and (lib / "github.com/crew/beta/repo.json").is_file(), "every project of the account is added", 60)
        wait_for(idle, "adding the account's projects finishes", 60)
        check(not (lib / "github.com/crew/old-fork").exists(), "its forks are left out")
        accounts = json.loads((lib / "_library/accounts.json").read_text())["accounts"]
        check([x["name"] for x in accounts] == ["crew"], "the account is followed")
        make_project("crew", "gamma", "# gamma\n")
        api("remove", {"id": "github.com/crew/beta"})
        api("check_all", {})
        wait_for(lambda: (lib / "github.com/crew/gamma/repo.json").is_file(), "a followed account's new project is added at the next check", 60)
        wait_for(idle, "the checks finish", 60)
        check(not (lib / "github.com/crew/beta").exists(), "a project removed by hand isn't added back")
        page.goto(BASE + "/#/settings")
        page.wait_for_selector("#accounts [data-account='github.com/crew']")
        page.click("#accounts >> text=Stop following")
        wait_for(lambda: page.locator("#accounts [data-account]").count() == 0, "an account can be unfollowed")
        page.goto(BASE + "/#/")

        # ---- dark, narrow, errors --------------------------------------------------------
        page.click("#theme-toggle")
        page.click("#theme-toggle")
        check(page.evaluate("document.documentElement.dataset.theme") == "dark", "the theme can be dark")
        page.screenshot(path=str(a.out / "09-dark.png"))
        page.set_viewport_size({"width": 860, "height": 700})
        page.goto(BASE + "/#/repo/github.com/demo/tool")
        page.wait_for_selector("#readme")
        check(page.evaluate("document.documentElement.scrollWidth <= innerWidth + 1"), "a narrow window doesn't scroll sideways")
        page.screenshot(path=str(a.out / "10-narrow.png"))
        errors = page.evaluate("window.__errors")
        check(not errors, f"no errors in the page {errors}")
        browser.close()
finally:
    serve.terminate()
    mock.terminate()

print(f"\n{len(failures)} failed" if failures else "\nall passed")
sys.exit(1 if failures else 0)
