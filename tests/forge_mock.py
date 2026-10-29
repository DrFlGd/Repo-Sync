#!/usr/bin/env python3
"""A stand-in for the parts of GitHub's and GitLab's APIs that Repo Sync uses,
serving local git repositories, so adding projects and scheduled checks can be
tested without the network.

    python3 tests/forge_mock.py --repos <folder> --port 8795
    REPOSYNC_API="github.com=http://127.0.0.1:8795;gitlab.com=http://127.0.0.1:8795/api/v4" reposync-cli serve ...

<folder>/<owner>__<name>/ is a git repository (its HEAD is the default branch,
"main"). Its releases are folders in <folder>/<owner>__<name>.releases/<tag>/:
the files in one are the release's files, and an optional release.json gives
{"name", "body", "prerelease", "published_at"} (the folder's time otherwise).
Newest published first. A test makes a new commit or a new release folder to
simulate a change upstream. The same projects answer as GitHub (/repos/...)
and as GitLab (/api/v4/projects/<owner>%2F<name>/...).
"""
import argparse
import json
import os
import subprocess
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("--repos", type=Path, required=True)
ap.add_argument("--port", type=int, default=8795)
a = ap.parse_args()
BASE = f"http://127.0.0.1:{a.port}"
# GET counts by path, for tests that check what was asked (/_mock/hits)
hits = {}


def git(repo, *args, text=True):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, check=True, text=text).stdout


def find(owner, name):
    p = a.repos / f"{owner}__{name}"
    return p if (p / ".git").exists() else None


def iso(t):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(t))


def releases(owner, name):
    d = a.repos / f"{owner}__{name}.releases"
    out = []
    if not d.is_dir():
        return out
    for r in d.iterdir():
        if not r.is_dir():
            continue
        meta = json.loads((r / "release.json").read_text()) if (r / "release.json").exists() else {}
        files = sorted(f for f in r.iterdir() if f.is_file() and f.name != "release.json")
        out.append({
            "tag": r.name,
            "name": meta.get("name", r.name),
            "body": meta.get("body", ""),
            "prerelease": meta.get("prerelease", False),
            "published_at": meta.get("published_at") or iso(r.stat().st_mtime),
            "files": files,
        })
    out.sort(key=lambda x: x["published_at"], reverse=True)
    return out


def branches(repo):
    return [{"name": b} for b in git(repo, "branch", "--format=%(refname:short)").split()]


def commit_json(repo, ref):
    sha, date, author, msg = git(repo, "log", "-1", "--format=%H%x00%cI%x00%an%x00%B", ref).strip("\n").split("\x00")
    return sha, date, author, msg


class H(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, code, body, ctype="application/json"):
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def zip_of(self, repo, owner, name, sha):
        data = subprocess.run(["git", "-C", str(repo), "archive", "--format=zip", f"--prefix={owner}-{name}-{sha[:7]}/", sha],
                              capture_output=True, check=True).stdout
        return self.send(200, data, "application/zip")

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        hits[url.path] = hits.get(url.path, 0) + 1
        q = urllib.parse.parse_qs(url.query)
        parts = [urllib.parse.unquote(p) for p in url.path.split("/") if p]
        if parts == ["_mock", "hits"]:
            return self.send(200, hits)
        if parts[:1] == ["files"] and len(parts) == 5:
            # a release's file: /files/<owner>/<name>/<tag>/<file>
            p = a.repos / f"{parts[1]}__{parts[2]}.releases" / parts[3] / parts[4]
            return self.send(200, p.read_bytes(), "application/octet-stream") if p.is_file() else self.send(404, {"message": "Not Found"})
        if parts[:2] == ["api", "v4"] and len(parts) >= 4 and parts[2] == "projects":
            return self.gitlab(parts[3], parts[4:], q)
        if len(parts) >= 3 and parts[0] == "repos":
            return self.github(parts[1], parts[2], parts[3:])
        self.send(404, {"message": "Not Found"})

    def github(self, owner, name, rest):
        repo = find(owner, name)
        if not repo:
            return self.send(404, {"message": "Not Found"})
        head = git(repo, "rev-parse", "HEAD").strip()
        if not rest:
            return self.send(200, {
                "name": name, "full_name": f"{owner}/{name}", "html_url": f"https://github.com/{owner}/{name}",
                "description": f"{name}, a test project", "homepage": "", "default_branch": "main",
                "stargazers_count": 42, "forks_count": 3, "language": "Rust", "topics": ["testing", "demo"],
                "license": {"spdx_id": "MIT", "name": "MIT License"}, "archived": False,
                "pushed_at": iso(int(git(repo, "log", "-1", "--format=%ct").strip())), "created_at": "2024-01-01T00:00:00Z",
            })
        if rest[0] == "commits" and len(rest) >= 2:
            ref = "/".join(rest[1:])
            try:
                sha, date, author, msg = commit_json(repo, head if ref in ("main", "HEAD") else ref)
            except subprocess.CalledProcessError:
                return self.send(404, {"message": "No commit found"})
            return self.send(200, {"sha": sha, "commit": {"committer": {"date": date}, "author": {"name": author}, "message": msg}})
        if rest == ["branches"]:
            return self.send(200, branches(repo))
        if rest[0] == "zipball" and len(rest) == 2:
            return self.zip_of(repo, owner, name, rest[1])
        if rest[0] == "releases":
            rels = [self.gh_release(owner, name, r) for r in releases(owner, name)]
            if rest[1:] == ["latest"]:
                stable = [r for r in rels if not r["prerelease"]]
                return self.send(200, stable[0]) if stable else self.send(404, {"message": "Not Found"})
            return self.send(200, rels)
        self.send(404, {"message": "Not Found"})

    def gh_release(self, owner, name, r):
        return {
            "tag_name": r["tag"], "name": r["name"], "body": r["body"], "prerelease": r["prerelease"], "draft": False,
            "published_at": r["published_at"], "html_url": f"https://github.com/{owner}/{name}/releases/tag/{r['tag']}",
            "assets": [{"name": f.name, "size": f.stat().st_size, "updated_at": r["published_at"],
                        "url": f"{BASE}/files/{owner}/{name}/{r['tag']}/{urllib.parse.quote(f.name)}"} for f in r["files"]],
        }

    def gitlab(self, project, rest, q):
        owner, _, name = project.rpartition("/")
        repo = find(owner, name)
        if not repo:
            return self.send(404, {"message": "404 Project Not Found"})
        head = git(repo, "rev-parse", "HEAD").strip()
        if not rest:
            return self.send(200, {
                "name": name, "path_with_namespace": project, "web_url": f"https://gitlab.com/{project}",
                "description": f"{name}, a test project on GitLab", "default_branch": "main", "star_count": 7, "forks_count": 1,
                "topics": ["gitlab"], "license": {"nickname": "GPLv3", "name": "GNU GPL v3"}, "archived": False,
                "last_activity_at": iso(int(git(repo, "log", "-1", "--format=%ct").strip())), "created_at": "2024-01-01T00:00:00Z",
            })
        if rest == ["languages"]:
            return self.send(200, {"Python": 80.5, "Shell": 19.5})
        if rest[:2] == ["repository", "commits"] and len(rest) == 3:
            sha, date, author, msg = commit_json(repo, head if rest[2] in ("main", "HEAD") else rest[2])
            return self.send(200, {"id": sha, "committed_date": date, "author_name": author, "message": msg, "title": msg.splitlines()[0]})
        if rest == ["repository", "branches"]:
            return self.send(200, branches(repo))
        if rest == ["repository", "archive.zip"]:
            return self.zip_of(repo, owner, name, q.get("sha", [head])[0])
        if rest == ["releases"]:
            return self.send(200, [{
                "tag_name": r["tag"], "name": r["name"], "description": r["body"], "released_at": r["published_at"],
                "upcoming_release": False, "_links": {"self": f"https://gitlab.com/{project}/-/releases/{r['tag']}"},
                "assets": {"links": [{"name": f.name, "url": f"{BASE}/files/{owner}/{name}/{r['tag']}/{urllib.parse.quote(f.name)}"} for f in r["files"]]},
            } for r in releases(owner, name)])
        self.send(404, {"message": "404 Not Found"})


print(f"forge stand-in on {BASE}/ serving {a.repos}", flush=True)
ThreadingHTTPServer(("127.0.0.1", a.port), H).serve_forever()
