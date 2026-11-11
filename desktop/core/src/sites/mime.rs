//! One table of content types, by file extension, for hosted sites, the
//! `library://` protocol and `reposync-cli serve`.

/// The content type for a file name ("application/octet-stream" when unknown).
/// Text types say UTF-8.
pub fn for_path(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    };
    match ext.as_str() {
        "html" | "htm" | "xhtml" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" | "geojson" | "topojson" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json; charset=utf-8",
        "xml" | "rss" | "atom" => "application/xml; charset=utf-8",
        "txt" | "md" | "markdown" | "text" | "log" | "ini" | "toml" | "yml" | "yaml" | "glsl" | "frag" | "vert" | "wgsl" | "obj" | "mtl" => "text/plain; charset=utf-8",
        "csv" => "text/csv; charset=utf-8",
        "tsv" => "text/tab-separated-values; charset=utf-8",
        "svg" | "svgz" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" | "jfif" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "ktx2" => "image/ktx2",
        "basis" => "application/octet-stream",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",
        "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "mid" | "midi" => "audio/midi",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "vtt" => "text/vtt; charset=utf-8",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" => "application/gzip",
        "glb" => "model/gltf-binary",
        "gltf" => "model/gltf+json",
        "stl" => "model/stl",
        "usdz" => "model/vnd.usdz+zip",
        "bin" | "data" | "dat" => "application/octet-stream",
        _ => "application/octet-stream",
    }
}

/// Whether a content type is text the server may rewrite (addresses of saved files).
pub fn is_rewritable(ctype: &str) -> bool {
    let t = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    matches!(
        t.as_str(),
        "text/html" | "text/javascript" | "application/javascript" | "text/css" | "application/json" | "application/manifest+json" | "image/svg+xml" | "application/xml" | "text/xml"
    )
}

/// The kind of text, for scanning: "html", "js", "css", "json", "" for anything else.
pub fn text_kind(ctype: &str) -> &'static str {
    let t = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match t.as_str() {
        "text/html" | "application/xhtml+xml" => "html",
        "text/javascript" | "application/javascript" | "application/x-javascript" | "application/ecmascript" | "text/ecmascript" => "js",
        "text/css" => "css",
        "application/json" | "application/manifest+json" => "json",
        "image/svg+xml" => "svg",
        _ => "",
    }
}

/// An extension for a content type, for files saved without one (".html").
pub fn extension_for(ctype: &str) -> &'static str {
    let t = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    match t.as_str() {
        "text/html" => ".html",
        "text/css" => ".css",
        "text/javascript" | "application/javascript" => ".js",
        "application/json" => ".json",
        "application/manifest+json" => ".webmanifest",
        "application/wasm" => ".wasm",
        "image/svg+xml" => ".svg",
        "image/png" => ".png",
        "image/jpeg" => ".jpg",
        "image/webp" => ".webp",
        "image/gif" => ".gif",
        "font/woff2" => ".woff2",
        "font/woff" => ".woff",
        "font/ttf" => ".ttf",
        "text/plain" => ".txt",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_by_extension() {
        assert_eq!(for_path("a/b/three.module.min.js"), "text/javascript; charset=utf-8");
        assert_eq!(for_path("x.WASM"), "application/wasm");
        assert_eq!(for_path("guns/7.glb"), "model/gltf-binary");
        assert_eq!(for_path("music/vectors.mp3"), "audio/mpeg");
        assert_eq!(for_path("noext"), "application/octet-stream");
        assert_eq!(for_path(".hidden/README"), "application/octet-stream");
        assert!(is_rewritable("text/html; charset=utf-8") && !is_rewritable("image/png"));
        assert_eq!(text_kind("application/javascript"), "js");
        assert_eq!(extension_for("text/html; charset=utf-8"), ".html");
    }
}
