mod document;
mod encodings;
mod inspector;
pub mod template;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use document::Document;
use template::{Endian, Root};

#[derive(Default)]
struct AppState {
    doc: Mutex<Document>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocInfo {
    path: Option<String>,
    name: String,
    size: u64,
    dirty: bool,
    can_undo: bool,
    can_redo: bool,
}

#[derive(Serialize)]
struct EditResult {
    info: DocInfo,
    offset: Option<u64>,
    len: u64,
}

type CmdResult<T> = Result<T, String>;

fn info(app: &AppHandle, doc: &Document) -> DocInfo {
    let dirty = doc.dirty();
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_title(&format!("{}{} — HexEdit", doc.name(), if dirty { " *" } else { "" }));
    }
    DocInfo {
        path: doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
        name: doc.name(),
        size: doc.data.len() as u64,
        dirty,
        can_undo: doc.can_undo(),
        can_redo: doc.can_redo(),
    }
}

fn endian(little: bool) -> Endian {
    if little {
        Endian::Little
    } else {
        Endian::Big
    }
}

/// Asks before discarding unsaved changes. Must not run on the main thread.
fn confirm_discard(app: &AppHandle, state: &AppState) -> bool {
    let (dirty, name) = {
        let doc = state.doc.lock().unwrap();
        (doc.dirty(), doc.name())
    };
    !dirty
        || app
            .dialog()
            .message(format!("\"{name}\" has unsaved changes. Discard them?"))
            .title("Unsaved changes")
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom("Discard".into(), "Cancel".into()))
            .blocking_show()
}

// ---------------------------------------------------------------- document

#[tauri::command]
fn doc_info(app: AppHandle, state: State<AppState>) -> DocInfo {
    info(&app, &state.doc.lock().unwrap())
}

#[tauri::command]
async fn open_file(app: AppHandle, state: State<'_, AppState>, path: Option<String>) -> CmdResult<Option<DocInfo>> {
    if !confirm_discard(&app, &state) {
        return Ok(None);
    }
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => match app.dialog().file().set_title("Open file").blocking_pick_file() {
            Some(p) => p.into_path().map_err(|e| e.to_string())?,
            None => return Ok(None),
        },
    };
    let doc = Document::open(&path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let mut cur = state.doc.lock().unwrap();
    *cur = doc;
    Ok(Some(info(&app, &cur)))
}

#[tauri::command]
async fn new_file(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Option<DocInfo>> {
    if !confirm_discard(&app, &state) {
        return Ok(None);
    }
    let mut cur = state.doc.lock().unwrap();
    *cur = Document::default();
    Ok(Some(info(&app, &cur)))
}

#[derive(Serialize)]
struct SaveResult {
    info: DocInfo,
    /// backup created by this save
    backup: Option<String>,
}

#[tauri::command]
async fn save_file(
    app: AppHandle,
    state: State<'_, AppState>,
    save_as: bool,
    backup: bool,
) -> CmdResult<Option<SaveResult>> {
    let (existing, name) = {
        let doc = state.doc.lock().unwrap();
        (doc.path.clone(), doc.name())
    };
    let path = match existing {
        Some(p) if !save_as => p,
        _ => match app.dialog().file().set_title("Save as").set_file_name(&name).blocking_save_file() {
            Some(p) => p.into_path().map_err(|e| e.to_string())?,
            None => return Ok(None),
        },
    };
    let mut doc = state.doc.lock().unwrap();
    let bak = doc.save_to(&path, backup).map_err(|e| format!("Cannot save {}: {e}", path.display()))?;
    Ok(Some(SaveResult { info: info(&app, &doc), backup: bak.map(|b| b.display().to_string()) }))
}

#[tauri::command]
fn read_bytes(state: State<AppState>, offset: u64, len: u64) -> tauri::ipc::Response {
    let doc = state.doc.lock().unwrap();
    let start = (offset as usize).min(doc.data.len());
    let end = start.saturating_add(len as usize).min(doc.data.len());
    tauri::ipc::Response::new(doc.data[start..end].to_vec())
}

#[tauri::command]
fn write_bytes(
    app: AppHandle,
    state: State<AppState>,
    offset: u64,
    bytes: Vec<u8>,
    insert: bool,
    coalesce: bool,
) -> CmdResult<DocInfo> {
    let mut doc = state.doc.lock().unwrap();
    doc.write(offset as usize, &bytes, insert, coalesce)?;
    Ok(info(&app, &doc))
}

#[tauri::command]
fn delete_bytes(app: AppHandle, state: State<AppState>, offset: u64, len: u64) -> CmdResult<DocInfo> {
    let mut doc = state.doc.lock().unwrap();
    doc.delete(offset as usize, len as usize)?;
    Ok(info(&app, &doc))
}

#[tauri::command]
fn undo(app: AppHandle, state: State<AppState>) -> EditResult {
    let mut doc = state.doc.lock().unwrap();
    let t = doc.undo();
    EditResult { info: info(&app, &doc), offset: t.as_ref().map(|t| t.offset), len: t.map_or(0, |t| t.len) }
}

#[tauri::command]
fn redo(app: AppHandle, state: State<AppState>) -> EditResult {
    let mut doc = state.doc.lock().unwrap();
    let t = doc.redo();
    EditResult { info: info(&app, &doc), offset: t.as_ref().map(|t| t.offset), len: t.map_or(0, |t| t.len) }
}

#[derive(Serialize)]
struct FindResult {
    offset: u64,
    len: u64,
}

#[tauri::command]
fn find(
    state: State<AppState>,
    query: String,
    mode: String,
    encoding: String,
    little_endian: bool,
    from: u64,
    backwards: bool,
) -> CmdResult<Option<FindResult>> {
    let pat = encodings::search_pattern(&query, &mode, &encoding, little_endian)?;
    let doc = state.doc.lock().unwrap();
    let data = &doc.data;
    let n = pat.len();
    if n == 0 || n > data.len() {
        return Ok(None);
    }
    let from = (from as usize).min(data.len());
    let hit = if backwards {
        data[..(from + n - 1).min(data.len())].windows(n).rposition(|w| w == pat.as_slice())
    } else {
        data[from..].windows(n).position(|w| w == pat.as_slice()).map(|p| p + from)
    };
    Ok(hit.map(|o| FindResult { offset: o as u64, len: n as u64 }))
}

// --------------------------------------------------------------- inspector

#[tauri::command]
fn inspect(
    state: State<AppState>,
    offset: u64,
    sel_len: u64,
    little_endian: bool,
    encoding: String,
) -> Vec<inspector::Entry> {
    let doc = state.doc.lock().unwrap();
    let start = (offset as usize).min(doc.data.len());
    let window = (sel_len as usize).clamp(1024, 1 << 20);
    let end = start.saturating_add(window).min(doc.data.len());
    inspector::inspect(&doc.data[start..end], sel_len, little_endian, encodings::lookup(&encoding))
}

#[tauri::command]
fn inspector_write(
    app: AppHandle,
    state: State<AppState>,
    offset: u64,
    key: String,
    text: String,
    little_endian: bool,
) -> CmdResult<DocInfo> {
    let bytes = inspector::encode(&key, &text, little_endian)?;
    let mut doc = state.doc.lock().unwrap();
    doc.write(offset as usize, &bytes, false, false)?;
    Ok(info(&app, &doc))
}

#[tauri::command]
fn encoding_list() -> encodings::EncodingList {
    encodings::list()
}

#[tauri::command]
fn char_table(encoding: String) -> Vec<String> {
    encodings::char_table(encodings::lookup(&encoding))
}

// ---------------------------------------------------------------- templates

#[derive(Serialize)]
struct ParseResult {
    error: Option<template::TemplateError>,
    types: Vec<String>,
    has_instances: bool,
}

#[tauri::command]
fn template_parse(source: String) -> ParseResult {
    match template::parse(&source) {
        Ok(t) => ParseResult { error: None, types: t.root_names.clone(), has_instances: !t.instances.is_empty() },
        Err(e) => ParseResult { error: Some(e), types: vec![], has_instances: false },
    }
}

#[tauri::command]
fn template_apply(
    state: State<AppState>,
    source: String,
    root: Option<String>,
    offset: u64,
    little_endian: bool,
    encoding: String,
) -> CmdResult<template::Node> {
    let t = template::parse(&source).map_err(|e| e.to_string())?;
    let root = match root {
        Some(name) => Root::Type(name),
        None => Root::Instances,
    };
    let opts = template::Options { endian: endian(little_endian), encoding: encodings::lookup(&encoding) };
    let doc = state.doc.lock().unwrap();
    template::apply(&t, &doc.data, root, offset, &opts)
}

#[derive(Serialize)]
struct TemplateFile {
    path: String,
    text: String,
}

#[tauri::command]
async fn template_open(app: AppHandle) -> CmdResult<Option<TemplateFile>> {
    let Some(p) = app
        .dialog()
        .file()
        .set_title("Open template")
        .add_filter("C templates", &["h", "c", "hpp", "txt"])
        .add_filter("All files", &["*"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = p.into_path().map_err(|e| e.to_string())?;
    let text = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    Ok(Some(TemplateFile { path: path.to_string_lossy().into_owned(), text }))
}

#[tauri::command]
async fn template_save(app: AppHandle, path: Option<String>, text: String) -> CmdResult<Option<String>> {
    let path = match path {
        Some(p) => PathBuf::from(p),
        None => match app
            .dialog()
            .file()
            .set_title("Save template")
            .add_filter("C templates", &["h"])
            .set_file_name("template.h")
            .blocking_save_file()
        {
            Some(p) => p.into_path().map_err(|e| e.to_string())?,
            None => return Ok(None),
        },
    };
    std::fs::write(&path, text).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[derive(Serialize)]
struct Example {
    name: &'static str,
    text: &'static str,
}

#[tauri::command]
fn template_examples() -> Vec<Example> {
    vec![
        Example { name: "BMP image", text: include_str!("../../templates/bmp.h") },
        Example { name: "PNG chunks", text: include_str!("../../templates/png.h") },
        Example { name: "WAV audio", text: include_str!("../../templates/wav.h") },
        Example { name: "PE / MZ header", text: include_str!("../../templates/pe.h") },
        Example { name: "ZIP local header", text: include_str!("../../templates/zip.h") },
    ]
}

// ------------------------------------------------------------------- setup

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            if let Some(arg) = std::env::args_os().nth(1) {
                let path = PathBuf::from(arg);
                match Document::open(&path) {
                    Ok(doc) => *app.state::<AppState>().doc.lock().unwrap() = doc,
                    Err(e) => eprintln!("cannot open {}: {e}", path.display()),
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let (dirty, name) = {
                    let doc = state.doc.lock().unwrap();
                    (doc.dirty(), doc.name())
                };
                if dirty {
                    api.prevent_close();
                    let w = window.clone();
                    window
                        .dialog()
                        .message(format!("\"{name}\" has unsaved changes. Quit anyway?"))
                        .title("Unsaved changes")
                        .kind(MessageDialogKind::Warning)
                        .buttons(MessageDialogButtons::OkCancelCustom("Quit".into(), "Cancel".into()))
                        .show(move |quit| {
                            if quit {
                                let _ = w.destroy();
                            }
                        });
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            doc_info,
            open_file,
            new_file,
            save_file,
            read_bytes,
            write_bytes,
            delete_bytes,
            undo,
            redo,
            find,
            inspect,
            inspector_write,
            encoding_list,
            char_table,
            template_parse,
            template_apply,
            template_open,
            template_save,
            template_examples,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
