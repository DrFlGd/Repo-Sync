// Repo Sync desktop app: the front end in web/ in a window, with the library
// behind web/platform-desktop.js. Every command goes through
// reposync_core::api::App::call (the same table `reposync-cli serve` uses for
// testing); this file adds what needs the window: the folder dialog, the file
// manager, web links and the library:// protocol for pictures in readmes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use reposync_core::api::{App, AppPaths};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

struct AppState {
    app: Arc<App>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// One of the app's commands (see reposync_core::api), answered as JSON.
#[tauri::command]
async fn api(cmd: String, args: Value, state: State<'_, AppState>) -> Result<Value, String> {
    let app = state.app.clone();
    tauri::async_runtime::spawn_blocking(move || app.call(&cmd, args)).await.map_err(err)?
}

/// Pick a folder (the library). None if cancelled.
#[tauri::command]
async fn pick_folder(app: AppHandle, title: Option<String>) -> Result<Option<String>, String> {
    let picked = app.dialog().file().set_title(title.unwrap_or_else(|| "Choose a folder".into())).blocking_pick_folder();
    match picked {
        Some(p) => Ok(Some(p.into_path().map_err(err)?.display().to_string())),
        None => Ok(None),
    }
}

/// Show a file in the file manager.
#[tauri::command]
fn reveal(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(PathBuf::from(path)).map_err(err)
}

/// Open a folder (the library, a project's folder, a version) in the file manager.
#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(err)
}

/// Open a web link (a project's page, a release's page) in the default browser.
#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("Only web links open in the browser.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(err)
}

/// The app's own addresses. Anything else a link in a readme leads to opens in
/// the browser instead of replacing the app.
fn in_app(url: &tauri::Url) -> bool {
    match url.scheme() {
        "tauri" | "library" | "asset" | "ipc" | "about" | "blob" | "data" => true,
        "http" | "https" => url.host_str().is_some_and(|h| h == "localhost" || h == "127.0.0.1" || h.ends_with(".localhost")),
        _ => false,
    }
}

fn open_outside(app: &AppHandle, url: &tauri::Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.to_string(), None::<&str>);
    }
}

fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "json" => "application/json",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // library://localhost/<path> (http://library.localhost/<path> on Windows): files in the open library
        .register_asynchronous_uri_scheme_protocol("library", |ctx, request, responder| {
            let app = ctx.app_handle().state::<AppState>().app.clone();
            let path = request.uri().path().to_string();
            tauri::async_runtime::spawn_blocking(move || {
                let response = match app.library_file(&path) {
                    Ok(body) => tauri::http::Response::builder()
                        .status(200)
                        .header("Content-Type", mime(&path))
                        // an SVG from a readme is shown as a picture, never run as a page
                        .header("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'; img-src data:")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(body),
                    Err(e) => tauri::http::Response::builder().status(404).body(e.to_string().into_bytes()),
                };
                responder.respond(response.unwrap_or_else(|_| tauri::http::Response::new(Vec::new())));
            });
        })
        .setup(|app| {
            let paths = AppPaths {
                config_dir: app.path().app_config_dir()?,
                library_url: if cfg!(windows) { "http://library.localhost/".into() } else { "library://localhost/".into() },
            };
            let core = App::new(paths, std::time::Duration::from_secs(30))?;
            app.manage(AppState { app: core });
            // the window (tauri.conf.json, "create": false), made here so a link in a readme
            // can't take it over: the web opens in the browser, and no other windows open
            let cfg = app.config().app.windows.iter().find(|w| w.label == "main").cloned().ok_or("tauri.conf.json has no main window")?;
            let (to_browser, to_browser2) = (app.handle().clone(), app.handle().clone());
            tauri::WebviewWindowBuilder::from_config(app.handle(), &cfg)?
                .on_navigation(move |url| {
                    if in_app(url) {
                        return true;
                    }
                    open_outside(&to_browser, url);
                    false
                })
                .on_new_window(move |url, _features| {
                    if !in_app(&url) {
                        open_outside(&to_browser2, &url);
                    }
                    tauri::webview::NewWindowResponse::Deny
                })
                .build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![api, pick_folder, reveal, open_path, open_url])
        .run(tauri::generate_context!())
        .expect("error while running Repo Sync");
}
