#!/usr/bin/env python3
"""A stand-in for the servers a hosted site reaches: jsDelivr, Google Fonts
(whose stylesheet differs by browser, as the real one's does) and a live
website to capture, so saving sites for offline use can be tested without the
network.

    python3 tests/cdn_mock.py --site <folder> --port 8796
    REPOSYNC_VENDOR="cdn.jsdelivr.net=http://127.0.0.1:8796/jsdelivr;fonts.googleapis.com=http://127.0.0.1:8796/fonts;fonts.gstatic.com=http://127.0.0.1:8796/gstatic"

/live/... serves <folder> as a website (with /live/old redirecting to
/live/about.html); /_mock/hits counts the requests by path. A test changes a file
in <folder> to simulate the live site changing.
"""
import argparse
import json
import mimetypes
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ap = argparse.ArgumentParser()
ap.add_argument("--site", type=Path, required=True)
ap.add_argument("--port", type=int, default=8796)
a = ap.parse_args()
hits = {}

THREE = b"export const REVISION = '170';\nexport class Scene { constructor() { this.isScene = true; } }\n"
LOADER = b"import { Scene } from 'three';\nimport { REVISION } from '../../../build/three.module.min.js';\nexport class GLTFLoader { load() { return new Scene(); } }\nexport const R = REVISION;\n"
CHROME_CSS = b"@font-face { font-family: 'Chakra Petch'; font-weight: 500; src: url(https://fonts.gstatic.com/s/chakrapetch/v11/cIflMapbsEk7TDLdtEz1BwkWmpLJ.woff2) format('woff2'); }\n"
OLD_CSS = b"@font-face { font-family: 'Chakra Petch'; font-weight: 500; src: url(https://fonts.gstatic.com/s/chakrapetch/v11/old.ttf) format('truetype'); }\n"


class H(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def send(self, code, body, ctype="text/plain", extra=None):
        data = body if isinstance(body, bytes) else body.encode()
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(data)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(data)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        url = urllib.parse.urlparse(self.path)
        path = urllib.parse.unquote(url.path)
        hits[path] = hits.get(path, 0) + 1
        if path == "/_mock/hits":
            return self.send(200, json.dumps(hits), "application/json")
        if path == "/jsdelivr/npm/three@0.170.0/build/three.module.min.js":
            # jsDelivr sends some files as plain text; the app goes by the name
            return self.send(200, THREE, "text/plain; charset=utf-8")
        if path == "/jsdelivr/npm/three@0.170.0/examples/jsm/loaders/GLTFLoader.js":
            return self.send(200, LOADER, "application/javascript; charset=utf-8")
        if path == "/fonts/css2":
            chrome = "Chrome/" in self.headers.get("user-agent", "")
            return self.send(200, CHROME_CSS if chrome else OLD_CSS, "text/css; charset=utf-8")
        if path.startswith("/gstatic/s/chakrapetch/"):
            return self.send(200, b"wOF2-font-bytes", "font/woff2")
        if path == "/live" or path == "/live/old":
            return self.send(301, "", extra={"location": "/live/" if path == "/live" else "/live/about.html"})
        if path == "/robots.txt":
            return self.send(200, "User-agent: *\nDisallow: /live/private/\n")
        if path.startswith("/live/"):
            rel = path[len("/live/"):]
            p = (a.site / rel) if rel and not rel.endswith("/") else (a.site / rel / "index.html")
            if ".." in Path(rel).parts or not p.is_file():
                return self.send(404, "<h1>Not found</h1>", "text/html")
            ctype = mimetypes.guess_type(p.name)[0] or "application/octet-stream"
            return self.send(200, p.read_bytes(), ctype)
        self.send(404, "not found")


print(f"CDN and website stand-in on http://127.0.0.1:{a.port}/ serving {a.site}", flush=True)
ThreadingHTTPServer(("127.0.0.1", a.port), H).serve_forever()
