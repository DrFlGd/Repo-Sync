# Third-party code

| What | Used for | License |
| --- | --- | --- |
| [Preact](https://github.com/preactjs/preact) 10.27.2 @ `0dbe636` | Interface components (`web/vendor/preact`; copied by `tools/vendor_preact.py`, only import paths changed) | MIT |
| [htm](https://github.com/developit/htm) @ `d62dcfd` | JSX-like templates without a build step (`web/vendor/htm`) | Apache-2.0 |
| [ureq](https://crates.io/crates/ureq) (with rustls and webpki-roots) | Talking to GitHub, GitLab and Gitea servers, downloads | MIT or Apache-2.0 (webpki-roots: CDLA-Permissive-2.0) |
| [zip](https://crates.io/crates/zip), [flate2](https://crates.io/crates/flate2), [tar](https://crates.io/crates/tar) | Unpacking captured code and release archives | MIT or Apache-2.0 |
| [pulldown-cmark](https://crates.io/crates/pulldown-cmark) and [ammonia](https://crates.io/crates/ammonia) | Showing readmes and release notes, cleaned so nothing in them can run | MIT; ammonia: MIT or Apache-2.0 |
| [serde](https://crates.io/crates/serde), [serde_json](https://crates.io/crates/serde_json), [anyhow](https://crates.io/crates/anyhow) | Reading and writing the library's JSON | MIT or Apache-2.0 |
| [Overpass](https://github.com/google/fonts/tree/main/ofl/overpass) and [Overpass Mono](https://github.com/google/fonts/tree/main/ofl/overpassmono) | The interface's fonts (bundled into the app by `tools/build_desktop.py --fetch-fonts`) | SIL OFL 1.1 |
| [Tauri](https://tauri.app) 2 and its dialog and opener plugins | The desktop app's window | MIT or Apache-2.0 |

The app's shell, interface pieces, look (Drafting Sheet) and tests follow [Model Library](https://github.com/DrFlGd/Model-Library) and [SCAD Workshop](https://github.com/DrFlGd/SCAD-Workshop) (same owner).
