//! Telling whether a captured version is a website, which kind, where its files
//! are and what address it lives under. Reads only small files and never runs
//! anything (docs/PLAN.md, "Detecting a site").

use super::{BuildHint, SiteInfo, SiteKind};
use crate::sources::{Kind, PagesInfo, SourceRef};
use std::path::Path;

/// What the server said about the project's Pages, when it said anything.
#[derive(Clone, Debug, Default)]
pub struct Hints {
    pub has_pages: bool,
    pub pages: Option<PagesInfo>,
    /// The project's default branch, and the branch this version is of.
    pub default_branch: String,
    pub branch: String,
}

/// Work out what `dir` (one captured version of `source`) holds as a website.
pub fn detect(dir: &Path, source: &SourceRef, hints: &Hints) -> SiteInfo {
    let mut info = SiteInfo::default();
    if source.kind == Kind::Web {
        info.kind = SiteKind::Mirror;
        info.base = web_base(&source.path);
        info.entry = "index.html".into();
        info.evidence.push("Captured from the website".into());
        info.has_404 = dir.join("404.html").is_file();
        return info;
    }
    let workflows = read_workflows(dir);
    let pages_api = hints.pages.as_ref().filter(|p| !p.build_type.is_empty() || !p.branch.is_empty());

    // 1. a workflow that deploys to Pages
    for wf in &workflows {
        if let Some(step) = wf.steps.iter().find(|s| s.uses.starts_with("actions/upload-pages-artifact")) {
            let path = clean_rel(step.with("path").unwrap_or("_site"));
            let builds: Vec<&Step> = wf.steps.iter().filter(|s| s.run.as_deref().is_some_and(looks_like_build)).collect();
            let jekyll = wf.steps.iter().find(|s| s.uses.starts_with("actions/jekyll-build-pages"));
            if let Some(j) = jekyll {
                info.kind = SiteKind::Jekyll;
                info.root = clean_rel(j.with("source").unwrap_or("."));
                info.build = Some(BuildHint { tool: "jekyll".into(), command: "bundle exec jekyll build".into(), output: "_site".into() });
                info.evidence.push(format!("Built with Jekyll by {} when it's published", wf.name));
            } else if !builds.is_empty() || (!dir.join(&path).is_dir() && !path.is_empty()) {
                info.kind = SiteKind::Build;
                let command = builds.iter().filter_map(|s| s.run.as_deref()).flat_map(|r| r.lines()).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect::<Vec<_>>().join(" && ");
                info.build = Some(BuildHint { tool: tool_of(&command), command, output: path.clone() });
                info.evidence.push(format!("Built by {} when it's published (the site is the {} folder it makes)", wf.name, if path.is_empty() { "top" } else { &path }));
            } else {
                info.kind = SiteKind::Static;
                info.root = path.clone();
                info.evidence.push(format!("Published by {} from {}", wf.name, if path.is_empty() { "the whole repository".to_string() } else { format!("the {path} folder") }));
            }
            break;
        }
        if let Some(step) = wf.steps.iter().find(|s| s.uses.starts_with("peaceiris/actions-gh-pages") || s.uses.starts_with("JamesIves/github-pages-deploy-action")) {
            let peaceiris = step.uses.starts_with("peaceiris");
            let branch = step.with(if peaceiris { "publish_branch" } else { "branch" }).unwrap_or("gh-pages").to_string();
            let folder = clean_rel(step.with(if peaceiris { "publish_dir" } else { "folder" }).unwrap_or(if peaceiris { "public" } else { "build" }));
            let builds: Vec<&Step> = wf.steps.iter().filter(|s| s.run.as_deref().is_some_and(looks_like_build)).collect();
            if builds.is_empty() && dir.join(&folder).join("index.html").is_file() {
                info.kind = SiteKind::Static;
                info.root = folder;
            } else {
                info.kind = SiteKind::Branch;
                let command = builds.iter().filter_map(|s| s.run.as_deref()).flat_map(|r| r.lines()).map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect::<Vec<_>>().join(" && ");
                if !command.is_empty() {
                    info.build = Some(BuildHint { tool: tool_of(&command), command, output: folder });
                }
            }
            info.branch = branch.clone();
            info.evidence.push(format!("{} publishes the site to the {branch} branch", wf.name));
            break;
        }
    }
    // GitLab Pages: a job named "pages" whose artifacts are public/
    if !info.kind.is_site() {
        if let Ok(ci) = std::fs::read_to_string(dir.join(".gitlab-ci.yml")) {
            if ci.lines().any(|l| l.trim_end() == "pages:") {
                let builds = ci.lines().any(|l| looks_like_build(l.trim().trim_start_matches("- ")));
                if !builds && dir.join("public/index.html").is_file() {
                    info.kind = SiteKind::Static;
                    info.root = "public".into();
                } else {
                    info.kind = SiteKind::Build;
                    info.build = Some(BuildHint { tool: String::new(), command: String::new(), output: "public".into() });
                }
                info.evidence.push("Published by the pages job in .gitlab-ci.yml".into());
            }
        }
    }

    // 2. what the server says (GitHub's Pages API), for a branch deployment
    if !info.kind.is_site() {
        if let Some(p) = pages_api {
            if p.build_type == "workflow" {
                info.evidence.push("GitHub publishes it with a workflow".into());
            } else if !p.branch.is_empty() && p.branch != hints.branch && !hints.branch.is_empty() {
                info.kind = SiteKind::Branch;
                info.branch = p.branch.clone();
                info.evidence.push(format!("GitHub publishes it from the {} branch", p.branch));
            } else {
                let root = clean_rel(&p.path);
                info.root = root.clone();
                branch_site(dir, &mut info);
                info.evidence.push(format!("GitHub publishes it from {}", if root.is_empty() { "the top of the branch".to_string() } else { format!("the {root} folder") }));
            }
        }
    }

    // 3. the files themselves
    if !info.kind.is_site() {
        if let Some(b) = tool_build(dir) {
            info.kind = SiteKind::Build;
            info.evidence.push(format!("{} builds the site (into {})", b.tool, b.output));
            info.build = Some(b);
        } else if dir.join("index.html").is_file() && !vite_source(&dir.join("index.html")) {
            branch_site(dir, &mut info);
            info.evidence.push("index.html at the top of the repository".into());
        } else if dir.join("docs/index.html").is_file() || (hints.has_pages && dir.join("docs/_config.yml").is_file()) {
            info.root = "docs".into();
            branch_site(dir, &mut info);
            info.evidence.push("A site in the docs folder".into());
        } else if hints.has_pages {
            branch_site(dir, &mut info);
            info.evidence.push("GitHub Pages is turned on for it".into());
        }
    }
    if dir.join("index.html").is_file() && vite_source(&dir.join("index.html")) && info.kind == SiteKind::Static && info.root.is_empty() {
        // a Vite app's source, not the built site
        info.kind = SiteKind::Build;
        info.build = Some(BuildHint { tool: "npm".into(), command: "npm install && npm run build".into(), output: "dist".into() });
        info.evidence.push("index.html loads the app's source (/src/main…), so it has to be built".into());
    }
    if !info.kind.is_site() {
        return info;
    }

    let root = dir.join(&info.root);
    info.nojekyll = root.join(".nojekyll").is_file() || dir.join(".nojekyll").is_file();
    if info.kind == SiteKind::JekyllPassthrough && info.nojekyll {
        info.kind = SiteKind::Static;
        info.evidence.push(".nojekyll: served as plain files".into());
    }
    info.cname = std::fs::read_to_string(root.join("CNAME")).map(|s| s.lines().next().unwrap_or("").trim().to_string()).unwrap_or_default();
    if info.cname.is_empty() {
        if let Some(c) = pages_api.map(|p| p.cname.clone()).filter(|c| !c.is_empty()) {
            info.cname = c;
        }
    }
    info.base = base_path(source, &info.cname);
    info.has_404 = root.join("404.html").is_file();
    info.entry = if root.join("index.html").is_file() {
        "index.html".into()
    } else if root.join("index.htm").is_file() {
        "index.htm".into()
    } else if info.kind == SiteKind::JekyllPassthrough {
        ["index.md", "README.md", "readme.md", "Readme.md"].iter().find(|f| root.join(f).is_file()).map(|s| s.to_string()).unwrap_or_default()
    } else {
        String::new()
    };
    if info.has_404 {
        let text = std::fs::read_to_string(root.join("404.html")).unwrap_or_default();
        if text.contains("pathSegmentsToKeep") || text.contains("l.pathname.split('/')") || text.contains("?/") && text.contains("location.replace") {
            info.spa = true;
            info.evidence.push("404.html sends deep links back to the app (a single-page app)".into());
        }
    }
    if info.kind.servable() && lfs_pointers(&root) {
        info.warnings.push("Some files are kept in Git LFS, so the downloaded code has only pointers to them; those files won't load.".into());
    }
    info
}

/// The kind of site a published branch (gh-pages) holds.
pub fn detect_branch(dir: &Path, source: &SourceRef) -> SiteInfo {
    let mut info = SiteInfo::default();
    branch_site(dir, &mut info);
    info.evidence.push("The branch the site is published to".into());
    info.nojekyll = dir.join(".nojekyll").is_file();
    if info.nojekyll {
        info.kind = SiteKind::Static;
    }
    info.cname = std::fs::read_to_string(dir.join("CNAME")).map(|s| s.lines().next().unwrap_or("").trim().to_string()).unwrap_or_default();
    info.base = base_from_html(dir, source).unwrap_or_else(|| base_path(source, &info.cname));
    info.has_404 = dir.join("404.html").is_file();
    info.entry = if dir.join("index.html").is_file() { "index.html".into() } else { String::new() };
    info
}

/// The base path a built site expects, read from the root-relative addresses
/// its index.html uses ("/name/assets/…" for "/name/", "/assets/…" for "/").
pub fn base_from_html(dir: &Path, source: &SourceRef) -> Option<String> {
    let html = std::fs::read_to_string(dir.join("index.html")).ok()?;
    let project = format!("/{}/", source.name());
    let mut root_relative = 0;
    for r in super::scan::html_refs(&html) {
        if r.url.starts_with('/') && !r.url.starts_with("//") {
            if r.url.starts_with(&project) || r.url.eq_ignore_ascii_case(project.trim_end_matches('/')) {
                return Some(project);
            }
            if r.kind != super::scan::RefKind::Page {
                root_relative += 1;
            }
        }
    }
    (root_relative > 0).then(|| "/".to_string())
}

/// A site published from a branch: Jekyll runs on it unless .nojekyll says not to.
fn branch_site(dir: &Path, info: &mut SiteInfo) {
    let root = dir.join(&info.root);
    if jekyll_features(&root) {
        info.kind = SiteKind::Jekyll;
        info.build = Some(BuildHint { tool: "jekyll".into(), command: "bundle exec jekyll build".into(), output: "_site".into() });
        info.evidence.push("It uses Jekyll's layouts or front matter, so it's built when published".into());
    } else {
        info.kind = SiteKind::JekyllPassthrough;
    }
}

/// The address a site lives under on github.io (and GitLab and Codeberg Pages).
pub fn base_path(source: &SourceRef, cname: &str) -> String {
    if !cname.is_empty() {
        return "/".into();
    }
    let name = source.name().to_ascii_lowercase();
    let owner = source.owner().to_ascii_lowercase();
    let user_site = match source.kind {
        Kind::GitHub => name == format!("{owner}.github.io") || name == format!("{owner}.github.com"),
        Kind::GitLab => name == format!("{owner}.gitlab.io"),
        Kind::Gitea => name == "pages" || name == format!("{owner}.codeberg.page"),
        Kind::Web => false,
    };
    if user_site {
        "/".into()
    } else {
        format!("/{}/", source.name())
    }
}

/// A mirrored website's base path: the folder of its start address ("/vectorclad/").
pub fn web_base(path: &str) -> String {
    // path is "<domain>/<slug>"; the start address itself is in the site settings
    let _ = path;
    "/".into()
}

fn clean_rel(p: &str) -> String {
    let p = p.trim().trim_matches(['"', '\'']).trim();
    let p = p.strip_prefix("${{ github.workspace }}").unwrap_or(p);
    let parts: Vec<&str> = p.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect();
    if parts.iter().any(|s| *s == "..") {
        return String::new();
    }
    parts.join("/")
}

fn looks_like_build(run: &str) -> bool {
    let words = ["npm ", "npx ", "pnpm ", "yarn", "bun ", "hugo", "jekyll", "bundle ", "mkdocs", "make", "cargo ", "zola", "eleventy", "gatsby", "astro", "vite", "next ", "python", "pip ", "sphinx", "mdbook", "deno ", "trunk", "wasm-pack", "gulp", "grunt", "webpack"];
    run.lines().map(str::trim).any(|l| !l.starts_with('#') && words.iter().any(|w| l.starts_with(w) || l.contains(&format!(" {w}")) || l.contains(&format!("&& {w}"))))
}

fn tool_of(command: &str) -> String {
    for (needle, tool) in [("hugo", "hugo"), ("jekyll", "jekyll"), ("mkdocs", "mkdocs"), ("pnpm", "pnpm"), ("yarn", "yarn"), ("bun ", "bun"), ("npm", "npm"), ("npx", "npm"), ("zola", "zola"), ("mdbook", "mdbook"), ("make", "make"), ("cargo", "cargo"), ("python", "python")] {
        if command.contains(needle) {
            return tool.into();
        }
    }
    String::new()
}

/// A build described by the project's own files (no workflow): package.json with
/// a site generator, Hugo, MkDocs.
fn tool_build(dir: &Path) -> Option<BuildHint> {
    if let Ok(text) = std::fs::read_to_string(dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
            let has_build = v["scripts"]["build"].is_string();
            let dep = |name: &str| v["dependencies"][name].is_string() || v["devDependencies"][name].is_string();
            let pm = if dir.join("pnpm-lock.yaml").is_file() { "pnpm" } else if dir.join("yarn.lock").is_file() { "yarn" } else { "npm" };
            let install = match pm {
                "pnpm" => "pnpm install",
                "yarn" => "yarn install",
                _ if dir.join("package-lock.json").is_file() => "npm ci",
                _ => "npm install",
            };
            let output = if dep("next") {
                Some("out")
            } else if dep("react-scripts") || dep("@docusaurus/core") || dep("@sveltejs/adapter-static") {
                Some("build")
            } else if dep("vite") || dep("astro") || dep("@vue/cli-service") || dep("parcel") || dep("@11ty/eleventy") {
                Some(if dep("@11ty/eleventy") { "_site" } else { "dist" })
            } else {
                None
            };
            if let (true, Some(out)) = (has_build, output) {
                let run = if pm == "npm" { "npm run build".to_string() } else { format!("{pm} build") };
                return Some(BuildHint { tool: pm.into(), command: format!("{install} && {run}"), output: out.into() });
            }
        }
    }
    let hugo_cfg = ["hugo.toml", "hugo.yaml", "hugo.json"].iter().any(|f| dir.join(f).is_file())
        || (dir.join("config.toml").is_file() && (dir.join("content").is_dir() || dir.join("layouts").is_dir()) && dir.join("archetypes").is_dir());
    if hugo_cfg {
        return Some(BuildHint { tool: "hugo".into(), command: "hugo --minify".into(), output: "public".into() });
    }
    if dir.join("mkdocs.yml").is_file() || dir.join("mkdocs.yaml").is_file() {
        return Some(BuildHint { tool: "mkdocs".into(), command: "mkdocs build".into(), output: "site".into() });
    }
    if dir.join("book.toml").is_file() && dir.join("src/SUMMARY.md").is_file() {
        return Some(BuildHint { tool: "mdbook".into(), command: "mdbook build".into(), output: "book".into() });
    }
    None
}

/// index.html of a Vite (or similar) app's source: it loads /src/main.(t|j)sx?.
fn vite_source(index: &Path) -> bool {
    let html = std::fs::read_to_string(index).unwrap_or_default();
    super::scan::html_refs(&html).iter().any(|r| {
        let u = r.url.trim_start_matches('.');
        (u.starts_with("/src/") || u.starts_with("/app/")) && [".ts", ".tsx", ".jsx", ".vue", ".svelte"].iter().any(|e| u.ends_with(e)) || u == "/src/main.js"
    })
}

/// Jekyll's own features: a config with layouts, _layouts/_includes, or front matter.
fn jekyll_features(root: &Path) -> bool {
    if root.join("_layouts").is_dir() || root.join("_includes").is_dir() || root.join("_posts").is_dir() || root.join("_sass").is_dir() {
        return true;
    }
    if let Ok(g) = std::fs::read_to_string(root.join("Gemfile")) {
        if g.contains("jekyll") || g.contains("github-pages") {
            return true;
        }
    }
    // front matter in the site's own pages (a few are enough to tell)
    let mut seen = 0;
    let Ok(rd) = std::fs::read_dir(root) else { return false };
    for e in rd.flatten() {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if !(n.ends_with(".html") || n.ends_with(".md")) || n.starts_with("readme") {
            continue;
        }
        seen += 1;
        if seen > 40 {
            break;
        }
        if let Ok(mut f) = std::fs::File::open(e.path()) {
            use std::io::Read;
            let mut head = [0u8; 8];
            let n = f.read(&mut head).unwrap_or(0);
            if head[..n].starts_with(b"---\n") || head[..n].starts_with(b"---\r\n") {
                return true;
            }
        }
    }
    false
}

/// Whether the site's files include Git LFS pointer files (the ZIP has pointers, not the files).
fn lfs_pointers(root: &Path) -> bool {
    let mut stack = vec![(root.to_path_buf(), 0)];
    let mut looked = 0;
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if ft.is_dir() && depth < 6 && !name.starts_with('.') && name != "node_modules" {
                stack.push((e.path(), depth + 1));
            } else if ft.is_file() {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                if (100..300).contains(&len) {
                    looked += 1;
                    if std::fs::read(e.path()).map(|b| b.starts_with(b"version https://git-lfs.github.com/spec/v1")).unwrap_or(false) {
                        return true;
                    }
                }
                if looked > 2000 {
                    return false;
                }
            }
        }
    }
    false
}

/// One step of a workflow job, as far as detection needs it.
#[derive(Debug, Default)]
struct Step {
    uses: String,
    run: Option<String>,
    with: Vec<(String, String)>,
}

impl Step {
    fn with(&self, key: &str) -> Option<&str> {
        self.with.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

#[derive(Debug)]
struct Workflow {
    name: String,
    steps: Vec<Step>,
}

fn read_workflows(dir: &Path) -> Vec<Workflow> {
    let wf_dir = dir.join(".github/workflows");
    let Ok(rd) = std::fs::read_dir(&wf_dir) else { return vec![] };
    let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml")).collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|p| {
            let text = std::fs::read_to_string(&p).ok()?;
            Some(Workflow { name: p.file_name()?.to_string_lossy().into_owned(), steps: parse_steps(&text) })
        })
        .collect()
}

/// The steps of every job in a workflow file. Not a YAML parser: steps are the
/// "- " items under a `steps:` key, read for `uses:`, `run:` (one line or a
/// block) and the keys under `with:`.
fn parse_steps(text: &str) -> Vec<Step> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end()).collect();
    let indent = |l: &str| l.len() - l.trim_start().len();
    let blank = |l: &str| l.trim().is_empty() || l.trim_start().starts_with('#');
    let mut steps = vec![];
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() != "steps:" {
            i += 1;
            continue;
        }
        let base = indent(lines[i]);
        i += 1;
        // the block: lines deeper than `steps:`, or "- " items at its own depth
        let mut block: Vec<&str> = vec![];
        while i < lines.len() {
            let l = lines[i];
            if !blank(l) && (indent(l) < base || (indent(l) == base && !l.trim_start().starts_with("- "))) {
                break;
            }
            if !blank(l) {
                block.push(l);
            }
            i += 1;
        }
        let Some(item) = block.iter().find(|l| l.trim_start().starts_with("- ")).map(|l| indent(l)) else { continue };
        // split into items, each turned into plain "key: value" lines at one depth
        let mut items: Vec<Vec<(usize, String)>> = vec![];
        for l in block {
            let ind = indent(l);
            let t = l.trim_start();
            if ind == item && t.starts_with("- ") {
                let rest = t[2..].trim_start();
                items.push(vec![(item + 2, rest.to_string())]);
            } else if let Some(cur) = items.last_mut() {
                cur.push((ind, t.to_string()));
            }
        }
        for lines in items {
            steps.push(read_step(&lines, item + 2));
        }
    }
    steps
}

fn read_step(lines: &[(usize, String)], key_indent: usize) -> Step {
    let unquote = |v: &str| {
        let v = if v.starts_with('"') || v.starts_with('\'') { v } else { v.split(" #").next().unwrap_or(v) };
        v.trim().trim_matches(['"', '\'']).to_string()
    };
    let mut step = Step::default();
    let mut j = 0;
    while j < lines.len() {
        let (ind, t) = (&lines[j].0, lines[j].1.as_str());
        j += 1;
        if *ind != key_indent && j > 1 {
            continue;
        }
        let Some((k, v)) = t.split_once(':') else { continue };
        let (k, v) = (k.trim(), v.trim());
        let mut nested = vec![];
        while j < lines.len() && lines[j].0 > key_indent {
            nested.push(lines[j].1.as_str());
            j += 1;
        }
        match k {
            "uses" => step.uses = unquote(v),
            "run" if v.starts_with('|') || v.starts_with('>') || v.is_empty() => step.run = Some(nested.join("\n")),
            "run" => step.run = Some(unquote(v)),
            "with" => {
                for n in nested {
                    if let Some((wk, wv)) = n.split_once(':') {
                        step.with.push((wk.trim().to_string(), unquote(wv.trim())));
                    }
                }
            }
            _ => {}
        }
    }
    step
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("reposync-detect-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn put(d: &Path, rel: &str, text: &str) {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn gh(path: &str) -> SourceRef {
        SourceRef { kind: Kind::GitHub, host: "github.com".into(), path: path.into() }
    }

    const PAGES_YML: &str = "name: Pages\non: push\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@v4\n      - uses: actions/configure-pages@v5\n      - uses: actions/upload-pages-artifact@v3\n        with:\n          path: .\n      - id: deployment\n        uses: actions/deploy-pages@v4\n";

    #[test]
    fn an_actions_deployment_of_plain_files() {
        let d = tmp("actions");
        put(&d, ".github/workflows/pages.yml", PAGES_YML);
        put(&d, "index.html", "<html><script type=module src=js/main.js></script></html>");
        let i = detect(&d, &gh("Agilancer/vectorclad"), &Hints::default());
        assert_eq!(i.kind, SiteKind::Static);
        assert_eq!(i.root, "");
        assert_eq!(i.base, "/vectorclad/");
        assert_eq!(i.entry, "index.html");
        assert!(i.evidence[0].contains("pages.yml") && i.evidence[0].contains("whole repository"), "{:?}", i.evidence);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn workflows_of_every_shape() {
        let d = tmp("public");
        put(&d, ".github/workflows/site.yaml", "jobs:\n  b:\n    steps:\n    - uses: actions/checkout@v4\n    - name: Upload\n      uses: \"actions/upload-pages-artifact@v3\"\n      with:\n        path: ./public   # the site\n");
        put(&d, "public/index.html", "x");
        let i = detect(&d, &gh("o/n"), &Hints::default());
        assert_eq!((i.kind, i.root.as_str()), (SiteKind::Static, "public"));
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("vite");
        put(&d, ".github/workflows/deploy.yml", "jobs:\n  build:\n    steps:\n      - uses: actions/checkout@v4\n      - run: npm ci\n      - run: |\n          npm run build\n          echo done\n      - uses: actions/upload-pages-artifact@v3\n        with:\n          path: dist\n");
        put(&d, "index.html", "<script type=\"module\" src=\"/src/main.ts\"></script>");
        let i = detect(&d, &gh("o/app"), &Hints::default());
        assert_eq!(i.kind, SiteKind::Build, "{i:?}");
        let b = i.build.unwrap();
        assert_eq!((b.command.as_str(), b.output.as_str(), b.tool.as_str()), ("npm ci && npm run build && echo done", "dist", "npm"));
        let _ = std::fs::remove_dir_all(&d);

        // no path: upload-pages-artifact's default is _site
        let d = tmp("default");
        put(&d, ".github/workflows/p.yml", "jobs:\n  j:\n    steps:\n      - uses: actions/jekyll-build-pages@v1\n        with:\n          source: ./\n          destination: ./_site\n      - uses: actions/upload-pages-artifact@v3\n");
        let i = detect(&d, &gh("o/blog"), &Hints::default());
        assert_eq!(i.kind, SiteKind::Jekyll);
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("peaceiris");
        put(&d, ".github/workflows/gh.yml", "jobs:\n  j:\n    steps:\n      - run: hugo --minify\n      - uses: peaceiris/actions-gh-pages@v4\n        with:\n          publish_dir: ./public\n          publish_branch: site\n");
        let i = detect(&d, &gh("o/docs"), &Hints::default());
        assert_eq!((i.kind, i.branch.as_str()), (SiteKind::Branch, "site"));
        assert_eq!(i.build.unwrap().command, "hugo --minify");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn branch_deployments_and_base_paths() {
        let d = tmp("branch");
        put(&d, "index.html", "<html></html>");
        put(&d, "_drafts/x.html", "x");
        put(&d, "CNAME", "example.com\n");
        put(&d, "404.html", "<script>var pathSegmentsToKeep = 1; l.replace('?/')</script>");
        let i = detect(&d, &gh("o/n"), &Hints::default());
        assert_eq!(i.kind, SiteKind::JekyllPassthrough);
        assert_eq!((i.base.as_str(), i.cname.as_str()), ("/", "example.com"));
        assert!(i.has_404 && i.spa);
        put(&d, ".nojekyll", "");
        assert_eq!(detect(&d, &gh("o/n"), &Hints::default()).kind, SiteKind::Static);
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("jekyll");
        put(&d, "_config.yml", "title: x\n");
        put(&d, "_layouts/default.html", "{{ content }}");
        put(&d, "index.md", "---\nlayout: default\n---\n# Hi\n");
        let i = detect(&d, &gh("o/n"), &Hints { has_pages: true, ..Default::default() });
        assert_eq!(i.kind, SiteKind::Jekyll);
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("readme");
        put(&d, "README.md", "# Only a readme\n");
        assert_eq!(detect(&d, &gh("o/n"), &Hints::default()).kind, SiteKind::None, "a readme alone isn't a site");
        let i = detect(&d, &gh("o/n"), &Hints { has_pages: true, ..Default::default() });
        assert_eq!((i.kind, i.entry.as_str()), (SiteKind::JekyllPassthrough, "README.md"));
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("docs");
        put(&d, "docs/index.html", "x");
        let i = detect(&d, &gh("Someone/someone.github.io"), &Hints::default());
        assert_eq!((i.root.as_str(), i.base.as_str()), ("docs", "/"));
        let _ = std::fs::remove_dir_all(&d);

        let d = tmp("lfs");
        put(&d, "index.html", "x");
        put(&d, "big.glb", &format!("version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 123456\n", "a".repeat(64)));
        assert!(!detect(&d, &gh("o/n"), &Hints::default()).warnings.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn built_sites_from_their_own_files() {
        let d = tmp("pkg");
        put(&d, "package.json", r#"{"scripts":{"build":"vite build"},"devDependencies":{"vite":"^5"}}"#);
        put(&d, "package-lock.json", "{}");
        let i = detect(&d, &gh("o/n"), &Hints::default());
        assert_eq!(i.kind, SiteKind::Build);
        assert_eq!(i.build.unwrap().command, "npm ci && npm run build");
        let _ = std::fs::remove_dir_all(&d);
        let d = tmp("mk");
        put(&d, "mkdocs.yml", "site_name: x\n");
        assert_eq!(detect(&d, &gh("o/n"), &Hints::default()).build.unwrap().output, "site");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_base_a_built_site_expects() {
        let d = tmp("built");
        put(&d, "index.html", r#"<script type="module" src="/app/assets/index-abc.js"></script><a href="/">home</a>"#);
        assert_eq!(base_from_html(&d, &gh("o/app")).as_deref(), Some("/app/"));
        put(&d, "index.html", r#"<script type="module" src="/assets/index-abc.js"></script>"#);
        assert_eq!(base_from_html(&d, &gh("o/app")).as_deref(), Some("/"));
        put(&d, "index.html", r#"<script src="assets/index.js"></script>"#);
        assert_eq!(base_from_html(&d, &gh("o/app")), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}
