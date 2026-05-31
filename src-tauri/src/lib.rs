mod gallery;
mod models;
mod scanner;

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm,
};
use image::{codecs::jpeg::JpegEncoder, ImageFormat, ImageReader};
use models::*;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    io::{Read, Seek, SeekFrom},
    panic::{catch_unwind, AssertUnwindSafe},
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
    time::UNIX_EPOCH,
};
#[cfg(target_os = "macos")]
use tauri::TitleBarStyle;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    window::Color,
    Manager, Theme, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;
#[cfg(target_os = "windows")]
use tauri_plugin_dialog::{MessageDialogButtons, MessageDialogKind};

const SETTINGS_LABEL: &str = "settings";
pub(crate) const GALLERY_LABEL: &str = "gallery";
pub(crate) const CAROUSEL_LABEL: &str = "carousel";
const SETTINGS_MENU_ID: &str = "open_settings";
const GALLERY_MENU_ID: &str = "open_gallery";
const CAROUSEL_MENU_ID: &str = "open_carousel";
const QUIT_MENU_ID: &str = "quit";
pub(crate) const DEDUPE_MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
const EDITOR_SESSION_SEGMENT_TURNS: usize = 100;
const ENCRYPTED_XAI_KEY_PREFIX: &str = "enc:v1:";
const WINDOWS_CLOSE_BEHAVIOR_CONFIG_KEY: &str = "windows_close_behavior";
const WINDOWS_CLOSE_BEHAVIOR_ASK: &str = "ask";
const WINDOWS_CLOSE_BEHAVIOR_EXIT: &str = "exit";
const WINDOWS_CLOSE_BEHAVIOR_TRAY: &str = "tray";
#[allow(dead_code)]
pub(crate) const THUMBNAIL_MAX_EDGE: u32 = 768;
#[allow(dead_code)]
pub(crate) const THUMBNAIL_QUALITY: u8 = 82;
type ThumbnailProgressState = Arc<Mutex<ThumbnailProgress>>;

fn tray_icon() -> Option<tauri::image::Image<'static>> {
    let icon = image::load_from_memory(include_bytes!("../icons/bar-icon.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = icon.dimensions();
    Some(tauri::image::Image::new_owned(
        icon.into_raw(),
        width,
        height,
    ))
}

#[cfg(target_os = "windows")]
fn claim_single_instance() -> bool {
    use std::{
        ffi::{c_void, OsStr},
        os::windows::ffi::OsStrExt,
        ptr,
    };

    type Handle = *mut c_void;
    const ERROR_ALREADY_EXISTS: u32 = 183;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(
            lp_mutex_attributes: *mut c_void,
            b_initial_owner: i32,
            lp_name: *const u16,
        ) -> Handle;
        fn GetLastError() -> u32;
    }

    let name = OsStr::new("Local\\com.wang.gallery2.single-instance")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    unsafe {
        let handle = CreateMutexW(ptr::null_mut(), 1, name.as_ptr());
        !handle.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

#[cfg(not(target_os = "windows"))]
fn claim_single_instance() -> bool {
    true
}

fn initial_thumbnail_progress() -> ThumbnailProgress {
    ThumbnailProgress {
        running: false,
        stage: "idle".to_string(),
        processed: 0,
        total: 0,
        generated: 0,
        skipped: 0,
        message: String::new(),
        error: String::new(),
    }
}

pub(crate) fn db_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("Failed to resolve app data directory: {err}"))?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create app data directory: {err}"))?;
    Ok(dir.join("gallery.sqlite3"))
}

pub(crate) fn app_data_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|err| format!("Failed to resolve app data directory: {err}"))?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create app data directory: {err}"))?;
    Ok(dir)
}

fn user_path_string(path: &Path) -> String {
    clean_windows_verbatim_path(&path.to_string_lossy())
}

fn user_path_buf(path: PathBuf) -> PathBuf {
    PathBuf::from(user_path_string(&path))
}

fn canonical_user_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path)
        .map(user_path_buf)
        .map_err(|err| format!("Failed to canonicalize path {}: {err}", path.display()))
}

#[cfg(target_os = "windows")]
fn clean_windows_verbatim_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

#[cfg(not(target_os = "windows"))]
fn clean_windows_verbatim_path(path: &str) -> String {
    path.to_string()
}

fn editor_sessions_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("editor-sessions");
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create editor sessions directory: {err}"))?;
    Ok(dir)
}

fn editor_session_meta_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(editor_sessions_dir(app)?.join("meta.json"))
}

fn new_editor_session_id() -> String {
    let nanos = UNIX_EPOCH
        .elapsed()
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("session-{nanos}")
}

fn validate_editor_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || !session_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Invalid editor session id".to_string());
    }
    Ok(())
}

fn write_editor_session_meta(app: &tauri::AppHandle, session_id: &str) -> Result<(), String> {
    validate_editor_session_id(session_id)?;
    let meta = EditorSessionMeta {
        current_session_id: session_id.to_string(),
        updated_at: now_secs(),
    };
    let json = serde_json::to_vec_pretty(&meta)
        .map_err(|err| format!("Failed to serialize editor session metadata: {err}"))?;
    fs::write(editor_session_meta_path(app)?, json)
        .map_err(|err| format!("Failed to write editor session metadata: {err}"))?;
    Ok(())
}

fn current_editor_session_id(app: &tauri::AppHandle) -> Result<String, String> {
    let meta_path = editor_session_meta_path(app)?;
    if meta_path.exists() {
        let bytes = fs::read(&meta_path)
            .map_err(|err| format!("Failed to read editor session metadata: {err}"))?;
        if let Ok(meta) = serde_json::from_slice::<EditorSessionMeta>(&bytes) {
            if validate_editor_session_id(&meta.current_session_id).is_ok() {
                return Ok(meta.current_session_id);
            }
        }
    }

    let session_id = new_editor_session_id();
    write_editor_session_meta(app, &session_id)?;
    Ok(session_id)
}

pub(crate) fn default_generated_content_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("generated-content"))
}

pub(crate) fn default_thumbnail_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(app_data_dir(app)?.join("thumbnails"))
}

pub(crate) fn configured_generated_content_dir(
    app: &tauri::AppHandle,
    conn: &Connection,
) -> Result<PathBuf, String> {
    let app_dir = canonical_user_path(&app_data_dir(app)?)?;
    let default_dir = default_generated_content_dir(app)?;
    let stored = read_config(
        conn,
        "generated_content_dir",
        &user_path_string(&default_dir),
    )?;
    let dir = PathBuf::from(stored);
    let points_to_app_dir = dir == app_dir
        || canonical_user_path(&dir)
            .ok()
            .is_some_and(|canonical| canonical == app_dir);
    let normalized_dir = if points_to_app_dir {
        default_dir
    } else {
        user_path_buf(dir)
    };
    write_config(
        conn,
        "generated_content_dir",
        &user_path_string(&normalized_dir),
    )?;
    Ok(normalized_dir)
}

pub(crate) fn configured_thumbnail_dir(
    app: &tauri::AppHandle,
    conn: &Connection,
) -> Result<PathBuf, String> {
    let default_dir = default_thumbnail_dir(app)?;
    let stored = read_config(conn, "thumbnail_dir", &user_path_string(&default_dir))?;
    let dir = user_path_buf(PathBuf::from(stored));
    write_config(conn, "thumbnail_dir", &user_path_string(&dir))?;
    Ok(dir)
}

pub(crate) fn thumbnail_enabled(conn: &Connection) -> Result<bool, String> {
    Ok(read_config(conn, "thumbnail_enabled", "false")? == "true")
}

fn normalize_windows_close_behavior(value: String) -> String {
    match value.as_str() {
        WINDOWS_CLOSE_BEHAVIOR_EXIT | WINDOWS_CLOSE_BEHAVIOR_TRAY | WINDOWS_CLOSE_BEHAVIOR_ASK => {
            value
        }
        _ => WINDOWS_CLOSE_BEHAVIOR_ASK.to_string(),
    }
}

fn windows_close_behavior(conn: &Connection) -> Result<String, String> {
    Ok(normalize_windows_close_behavior(read_config(
        conn,
        WINDOWS_CLOSE_BEHAVIOR_CONFIG_KEY,
        WINDOWS_CLOSE_BEHAVIOR_ASK,
    )?))
}

fn persist_windows_close_behavior(
    app: &tauri::AppHandle,
    close_behavior: String,
) -> Result<String, String> {
    let conn = open_db(app)?;
    let close_behavior = normalize_windows_close_behavior(close_behavior);
    write_config(&conn, WINDOWS_CLOSE_BEHAVIOR_CONFIG_KEY, &close_behavior)?;
    Ok(close_behavior)
}

fn source_roots_from_conn(conn: &Connection) -> Result<Vec<PathBuf>, String> {
    let mut stmt = conn
        .prepare("SELECT path FROM source_paths ORDER BY path COLLATE NOCASE")
        .map_err(|err| format!("Failed to read source paths: {err}"))?;
    let roots = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("Failed to query source paths: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed to collect source paths: {err}"))?
        .into_iter()
        .map(|path| user_path_buf(PathBuf::from(path)))
        .collect();
    Ok(roots)
}

fn collapse_asset_roots(mut roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots.retain(|path| path.is_absolute());
    roots.sort();
    roots.dedup();
    roots.into_iter().fold(Vec::new(), |mut kept, root| {
        if !kept.iter().any(|parent: &PathBuf| root.starts_with(parent)) {
            kept.push(root);
        }
        kept
    })
}

fn allow_asset_directory(scope: &tauri::scope::fs::Scope, directory: &Path) -> Result<(), String> {
    let directory = if directory.exists() {
        let canonical = fs::canonicalize(directory).map_err(|err| {
            format!(
                "Failed to canonicalize asset directory {}: {err}",
                directory.display()
            )
        })?;
        if !canonical.is_dir() {
            return Ok(());
        }
        canonical
    } else {
        directory.to_path_buf()
    };

    if !directory.is_absolute() {
        return Ok(());
    }

    scope.allow_directory(&directory, true).map_err(|err| {
        format!(
            "Failed to allow asset directory {}: {err}",
            directory.display()
        )
    })
}

pub(crate) fn refresh_asset_scope_with_conn(
    app: &tauri::AppHandle,
    conn: &Connection,
) -> Result<(), String> {
    let mut roots = source_roots_from_conn(conn)?;
    roots.push(app_data_dir(app)?);
    roots.push(configured_generated_content_dir(app, conn)?);
    roots.push(configured_thumbnail_dir(app, conn)?);

    let scope = app.asset_protocol_scope();
    for root in collapse_asset_roots(roots) {
        allow_asset_directory(&scope, &root)?;
    }
    Ok(())
}

fn refresh_asset_scope(app: &tauri::AppHandle) -> Result<(), String> {
    let conn = open_db(app)?;
    refresh_asset_scope_with_conn(app, &conn)
}

pub(crate) fn open_db(app: &tauri::AppHandle) -> Result<Connection, String> {
    let conn =
        Connection::open(db_path(app)?).map_err(|err| format!("Failed to open database: {err}"))?;
    init_db(&conn)?;
    Ok(conn)
}

fn init_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS source_paths (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS images (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            media_type TEXT NOT NULL DEFAULT 'image',
            width INTEGER NOT NULL,
            height INTEGER NOT NULL,
            modified INTEGER NOT NULL,
            size INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS images_updated_at_idx ON images(updated_at DESC);
        CREATE INDEX IF NOT EXISTS images_sort_idx ON images(modified DESC, path COLLATE NOCASE ASC);

        CREATE TABLE IF NOT EXISTS app_config (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS image_thumbnails (
            image_path TEXT PRIMARY KEY NOT NULL,
            thumb_path TEXT NOT NULL,
            source_modified INTEGER NOT NULL,
            source_size INTEGER NOT NULL,
            width INTEGER NOT NULL,
            height INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        ",
    )
    .map_err(|err| format!("Failed to initialize database: {err}"))?;
    migrate_db(conn)
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|err| format!("Failed to inspect table {table}: {err}"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|err| format!("Failed to query table {table}: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed to collect table {table} columns: {err}"))?;
    Ok(columns)
}

fn migrate_db(conn: &Connection) -> Result<(), String> {
    let source_columns = table_columns(conn, "source_paths")?;
    if !source_columns.iter().any(|column| column == "id") {
        conn.execute_batch(
            "
            ALTER TABLE source_paths RENAME TO source_paths_old;
            CREATE TABLE source_paths (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                created_at INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO source_paths (path, created_at)
            SELECT path, created_at FROM source_paths_old ORDER BY path COLLATE NOCASE;
            DROP TABLE source_paths_old;
            ",
        )
        .map_err(|err| format!("Failed to migrate source paths: {err}"))?;
    }

    let image_columns = table_columns(conn, "images")?;
    if !image_columns.iter().any(|column| column == "id")
        || image_columns.iter().any(|column| column == "source_path")
        || !image_columns.iter().any(|column| column == "media_type")
    {
        conn.execute_batch(
            "
            ALTER TABLE images RENAME TO images_old;
            CREATE TABLE images (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                media_type TEXT NOT NULL DEFAULT 'image',
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                modified INTEGER NOT NULL,
                size INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            INSERT OR IGNORE INTO images (path, media_type, width, height, modified, size, updated_at)
            SELECT path, 'image', width, height, modified, size, updated_at
            FROM images_old
            ORDER BY modified DESC, path COLLATE NOCASE;
            DROP TABLE images_old;
            ",
        )
        .map_err(|err| format!("Failed to migrate media records: {err}"))?;
    }

    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS source_paths_path_idx ON source_paths(path)",
        [],
    )
    .map_err(|err| format!("Failed to index source paths: {err}"))?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS images_path_idx ON images(path)",
        [],
    )
    .map_err(|err| format!("Failed to index media paths: {err}"))?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS images_sort_idx ON images(modified DESC, path COLLATE NOCASE ASC)",
        [],
    )
    .map_err(|err| format!("Failed to index media sort order: {err}"))?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS image_thumbnails_updated_at_idx ON image_thumbnails(updated_at DESC)",
        [],
    )
    .map_err(|err| format!("Failed to index thumbnails: {err}"))?;
    Ok(())
}

pub(crate) fn read_config(
    conn: &Connection,
    key: &str,
    default_value: &str,
) -> Result<String, String> {
    match conn.query_row(
        "SELECT value FROM app_config WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => Ok(value),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(default_value.to_string()),
        Err(err) => Err(format!("Failed to read config {key}: {err}")),
    }
}

pub(crate) fn write_config(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "
        INSERT INTO app_config (key, value)
        VALUES (?1, ?2)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        ",
        params![key, value],
    )
    .map_err(|err| format!("Failed to write config {key}: {err}"))?;
    Ok(())
}

fn xai_encryption_key(app: &tauri::AppHandle) -> Result<[u8; 32], String> {
    let mut hasher = Sha256::new();
    hasher.update(b"gallery2:xai-key:v1");
    hasher.update(app_data_dir(app)?.to_string_lossy().as_bytes());
    hasher.update(env::var("USER").unwrap_or_default().as_bytes());
    hasher.update(env::var("USERNAME").unwrap_or_default().as_bytes());
    hasher.update(env::var("HOME").unwrap_or_default().as_bytes());
    hasher.update(env::var("USERPROFILE").unwrap_or_default().as_bytes());
    Ok(hasher.finalize().into())
}

fn encrypt_xai_key(app: &tauri::AppHandle, plaintext: &str) -> Result<String, String> {
    let key = xai_encryption_key(app)?;
    let cipher = Aes256Gcm::new(&key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| "Failed to encrypt xAI key".to_string())?;
    Ok(format!(
        "{ENCRYPTED_XAI_KEY_PREFIX}{}:{}",
        base64_encode(&nonce),
        base64_encode(&ciphertext)
    ))
}

fn decrypt_xai_key(app: &tauri::AppHandle, stored: &str) -> Result<String, String> {
    let Some(payload) = stored.strip_prefix(ENCRYPTED_XAI_KEY_PREFIX) else {
        return Ok(stored.to_string());
    };
    let (nonce, ciphertext) = payload
        .split_once(':')
        .ok_or_else(|| "Invalid encrypted xAI key".to_string())?;
    let nonce = base64_decode(nonce)?;
    let ciphertext = base64_decode(ciphertext)?;
    let key = xai_encryption_key(app)?;
    let cipher = Aes256Gcm::new(&key.into());
    let plaintext = cipher
        .decrypt(nonce.as_slice().into(), ciphertext.as_slice())
        .map_err(|_| "Failed to decrypt xAI key".to_string())?;
    String::from_utf8(plaintext).map_err(|_| "Invalid decrypted xAI key".to_string())
}

fn read_xai_key_config(app: &tauri::AppHandle, conn: &Connection) -> Result<String, String> {
    let stored = read_config(conn, "xai_key", "")?;
    if stored.is_empty() {
        Ok(stored)
    } else {
        decrypt_xai_key(app, &stored)
    }
}

fn write_xai_key_config(
    app: &tauri::AppHandle,
    conn: &Connection,
    xai_key: &str,
) -> Result<(), String> {
    let xai_key = xai_key.trim();
    let stored = if xai_key.is_empty() {
        String::new()
    } else {
        encrypt_xai_key(app, xai_key)?
    };
    write_config(conn, "xai_key", &stored)
}

fn normalize_gallery_preferences(has_gap: bool, theme: String) -> GalleryPreferences {
    GalleryPreferences {
        has_gap,
        theme: if theme == "black" {
            "black".to_string()
        } else {
            "white".to_string()
        },
        min_column_width: 280,
    }
}

fn get_gallery_preferences_from_conn(conn: &Connection) -> Result<GalleryPreferences, String> {
    let has_gap = read_config(conn, "gallery_has_gap", "false")? == "true";
    let theme = read_config(conn, "gallery_theme", "white")?;
    let mut preferences = normalize_gallery_preferences(has_gap, theme);
    preferences.min_column_width = read_config(conn, "min_column_width", "280")?
        .parse::<u32>()
        .unwrap_or(280)
        .clamp(100, 600);
    Ok(preferences)
}

fn get_gallery_preferences_from_app(app: &tauri::AppHandle) -> Result<GalleryPreferences, String> {
    let conn = open_db(app)?;
    get_gallery_preferences_from_conn(&conn)
}

fn normalize_gallery_mode(mode: String, preferences: &GalleryPreferences) -> String {
    match mode.as_str() {
        "gap" | "none" | "black" | "white" => mode,
        _ => {
            if !preferences.has_gap {
                "none".to_string()
            } else if preferences.theme == "black" {
                "black".to_string()
            } else {
                "white".to_string()
            }
        }
    }
}

fn current_platform() -> String {
    #[cfg(target_os = "windows")]
    {
        "windows".to_string()
    }
    #[cfg(target_os = "macos")]
    {
        "macos".to_string()
    }
    #[cfg(target_os = "linux")]
    {
        "linux".to_string()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        "unknown".to_string()
    }
}

fn gallery_background_color(preferences: &GalleryPreferences) -> Color {
    if preferences.theme == "black" {
        Color(26, 27, 30, 255)
    } else {
        Color(255, 255, 255, 255)
    }
}

fn apply_gallery_window_preferences(
    window: &WebviewWindow,
    preferences: &GalleryPreferences,
) -> Result<(), String> {
    let color = gallery_background_color(preferences);
    window
        .set_background_color(Some(color))
        .map_err(|err| format!("Failed to set gallery background color: {err}"))?;
    window
        .set_theme(Some(if preferences.theme == "black" {
            Theme::Dark
        } else {
            Theme::Light
        }))
        .map_err(|err| format!("Failed to set gallery theme: {err}"))?;
    Ok(())
}

fn is_gallery_window(label: &str) -> bool {
    label == GALLERY_LABEL || label == CAROUSEL_LABEL
}

fn now_secs() -> i64 {
    UNIX_EPOCH
        .elapsed()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn now_nanos() -> i64 {
    UNIX_EPOCH
        .elapsed()
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn metadata_secs(path: &Path) -> Result<(i64, i64), String> {
    let metadata = fs::metadata(path)
        .map_err(|err| format!("Failed to read metadata for {}: {err}", path.display()))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0);
    Ok((modified, metadata.len() as i64))
}

fn read_media_header(path: &Path) -> Result<Vec<u8>, String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
    let mut data = Vec::new();
    file.by_ref()
        .take(4 * 1024 * 1024)
        .read_to_end(&mut data)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    Ok(data)
}

fn read_exact_at(file: &mut fs::File, offset: u64, length: usize) -> Option<Vec<u8>> {
    let mut data = vec![0u8; length];
    file.seek(SeekFrom::Start(offset)).ok()?;
    file.read_exact(&mut data).ok()?;
    Some(data)
}

fn read_be_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_be_bytes)
}

fn read_be_u64(data: &[u8], offset: usize) -> Option<u64> {
    data.get(offset..offset + 8)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u64::from_be_bytes)
}

fn read_le_u32(data: &[u8], offset: usize) -> Option<u32> {
    data.get(offset..offset + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_le_bytes)
}

fn mp4_box_bounds(data: &[u8], start: usize, end: usize) -> Vec<(usize, usize, [u8; 4])> {
    let mut boxes = Vec::new();
    let mut cursor = start;
    while cursor + 8 <= end && cursor + 8 <= data.len() {
        let Some(size32) = read_be_u32(data, cursor) else {
            break;
        };
        let Some(kind) = data.get(cursor + 4..cursor + 8) else {
            break;
        };
        let mut header_size = 8usize;
        let mut box_size = size32 as u64;
        if size32 == 1 {
            let Some(size64) = read_be_u64(data, cursor + 8) else {
                break;
            };
            header_size = 16;
            box_size = size64;
        } else if size32 == 0 {
            box_size = (end - cursor) as u64;
        }
        if box_size < header_size as u64 {
            break;
        }
        let box_end = cursor
            .saturating_add(box_size as usize)
            .min(end)
            .min(data.len());
        if box_end <= cursor + header_size {
            break;
        }
        boxes.push((
            cursor + header_size,
            box_end,
            [kind[0], kind[1], kind[2], kind[3]],
        ));
        cursor = box_end;
    }
    boxes
}

fn find_mp4_boxes(data: &[u8], start: usize, end: usize, target: [u8; 4]) -> Vec<(usize, usize)> {
    mp4_box_bounds(data, start, end)
        .into_iter()
        .filter_map(|(content_start, content_end, kind)| {
            if kind == target {
                Some((content_start, content_end))
            } else {
                None
            }
        })
        .collect()
}

fn mp4_handler_type(data: &[u8], mdia_start: usize, mdia_end: usize) -> Option<[u8; 4]> {
    for (hdlr_start, hdlr_end) in find_mp4_boxes(data, mdia_start, mdia_end, *b"hdlr") {
        if hdlr_start + 12 <= hdlr_end {
            let bytes = data.get(hdlr_start + 8..hdlr_start + 12)?;
            return Some([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
    }
    None
}

fn mp4_tkhd_dimensions(data: &[u8], trak_start: usize, trak_end: usize) -> Option<(u32, u32)> {
    for (tkhd_start, tkhd_end) in find_mp4_boxes(data, trak_start, trak_end, *b"tkhd") {
        let version = *data.get(tkhd_start)?;
        let dimensions_offset = if version == 1 { 96 } else { 76 };
        if tkhd_start + dimensions_offset + 8 > tkhd_end {
            continue;
        }
        let width = read_be_u32(data, tkhd_start + dimensions_offset)? >> 16;
        let height = read_be_u32(data, tkhd_start + dimensions_offset + 4)? >> 16;
        if width > 0 && height > 0 {
            return Some((width, height));
        }
    }
    None
}

fn mp4_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    for (moov_start, moov_end) in find_mp4_boxes(data, 0, data.len(), *b"moov") {
        for (trak_start, trak_end) in find_mp4_boxes(data, moov_start, moov_end, *b"trak") {
            let is_video = find_mp4_boxes(data, trak_start, trak_end, *b"mdia")
                .into_iter()
                .any(|(mdia_start, mdia_end)| {
                    mp4_handler_type(data, mdia_start, mdia_end) == Some(*b"vide")
                });
            if is_video {
                if let Some(dimensions) = mp4_tkhd_dimensions(data, trak_start, trak_end) {
                    return Some(dimensions);
                }
            }
        }
    }
    None
}

fn mp4_dimensions_from_path(path: &Path) -> Option<(u32, u32)> {
    let mut file = fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let mut cursor = 0u64;
    while cursor + 8 <= file_len {
        let header = read_exact_at(&mut file, cursor, 8)?;
        let size32 = u32::from_be_bytes(header.get(0..4)?.try_into().ok()?);
        let kind = header.get(4..8)?;
        let mut header_size = 8u64;
        let mut box_size = u64::from(size32);
        if size32 == 1 {
            let size64 = read_exact_at(&mut file, cursor + 8, 8)?;
            box_size = u64::from_be_bytes(size64.get(0..8)?.try_into().ok()?);
            header_size = 16;
        } else if size32 == 0 {
            box_size = file_len.saturating_sub(cursor);
        }
        if box_size < header_size {
            return None;
        }
        if kind == b"moov" {
            let content_size = box_size.saturating_sub(header_size);
            if content_size > 128 * 1024 * 1024 {
                return None;
            }
            let mut moov = vec![0, 0, 0, 0, b'm', b'o', b'o', b'v'];
            let mut content =
                read_exact_at(&mut file, cursor + header_size, content_size as usize)?;
            moov.append(&mut content);
            let moov_len = moov.len() as u32;
            moov[0..4].copy_from_slice(&moov_len.to_be_bytes());
            return mp4_dimensions(&moov);
        }
        cursor = cursor.saturating_add(box_size);
    }
    None
}

fn read_ebml_vint(data: &[u8], offset: usize, strip_marker: bool) -> Option<(u64, usize)> {
    let first = *data.get(offset)?;
    let leading = first.leading_zeros() as usize;
    let length = leading + 1;
    if length > 8 || offset + length > data.len() {
        return None;
    }
    let mut value = if strip_marker {
        let marker_mask = if length == 8 { 0 } else { 0xff >> length };
        (first & marker_mask) as u64
    } else {
        first as u64
    };
    for byte in data.get(offset + 1..offset + length)? {
        value = (value << 8) | u64::from(*byte);
    }
    Some((value, length))
}

fn read_ebml_uint(data: &[u8]) -> u32 {
    data.iter()
        .fold(0u32, |value, byte| (value << 8) | u32::from(*byte))
}

fn ebml_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    let mut cursor = 0usize;
    let mut width = None;
    let mut height = None;
    while cursor < data.len() {
        let Some((id, id_len)) = read_ebml_vint(data, cursor, false) else {
            cursor += 1;
            continue;
        };
        let size_offset = cursor + id_len;
        let Some((size, size_len)) = read_ebml_vint(data, size_offset, true) else {
            cursor += 1;
            continue;
        };
        let value_start = size_offset + size_len;
        let value_end = value_start.saturating_add(size as usize);
        if value_end > data.len() {
            cursor += 1;
            continue;
        }
        if id == 0xb0 && size <= 4 {
            width = Some(read_ebml_uint(&data[value_start..value_end]));
        } else if id == 0xba && size <= 4 {
            height = Some(read_ebml_uint(&data[value_start..value_end]));
        }
        if let (Some(width), Some(height)) = (width, height) {
            if width > 0 && height > 0 {
                return Some((width, height));
            }
        }
        cursor = value_end.max(cursor + 1);
    }
    None
}

fn avi_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.get(0..4)? != b"RIFF" || data.get(8..12)? != b"AVI " {
        return None;
    }
    let mut cursor = 12usize;
    while cursor + 8 <= data.len() {
        let kind = data.get(cursor..cursor + 4)?;
        let size = read_le_u32(data, cursor + 4)? as usize;
        let content_start = cursor + 8;
        let content_end = content_start.saturating_add(size).min(data.len());
        if kind == b"avih" && content_start + 40 <= content_end {
            let width = read_le_u32(data, content_start + 32)?;
            let height = read_le_u32(data, content_start + 36)?;
            if width > 0 && height > 0 {
                return Some((width, height));
            }
        }
        cursor = content_end + (size % 2);
    }
    None
}

fn video_dimensions(path: &Path) -> Result<(u32, u32), String> {
    let data = read_media_header(path)?;
    mp4_dimensions_from_path(path)
        .or_else(|| mp4_dimensions(&data))
        .or_else(|| ebml_dimensions(&data))
        .or_else(|| avi_dimensions(&data))
        .ok_or_else(|| format!("Failed to read video dimensions for {}", path.display()))
}

pub(crate) fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn image_format(path: &Path) -> Option<ImageFormat> {
    ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .format()
}

pub(crate) fn image_format_extension(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Jpeg => Some("jpeg"),
        ImageFormat::Png => Some("png"),
        ImageFormat::WebP => Some("webp"),
        ImageFormat::Gif => Some("gif"),
        ImageFormat::Bmp => Some("bmp"),
        ImageFormat::Tiff => Some("tiff"),
        _ => None,
    }
}

fn image_mime_type(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Png => Some("image/png"),
        ImageFormat::WebP => Some("image/webp"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::Bmp => Some("image/bmp"),
        ImageFormat::Tiff => Some("image/tiff"),
        _ => None,
    }
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        encoded.push(TABLE[(first >> 2) as usize] as char);
        encoded.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut buffer = 0u32;
    let mut bits = 0u8;
    let mut decoded = Vec::with_capacity(input.len() * 3 / 4);
    for byte in input.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            _ => return Err("Invalid base64 data".to_string()),
        } as u32;
        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            decoded.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    Ok(decoded)
}

#[allow(dead_code)]
pub(crate) fn hex_encode(data: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(data.len() * 2);
    for byte in data {
        encoded.push(TABLE[(byte >> 4) as usize] as char);
        encoded.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[allow(dead_code)]
pub(crate) fn thumbnail_cache_key(path: &str, modified: i64, size: i64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_bytes());
    hasher.update(modified.to_le_bytes());
    hasher.update(size.to_le_bytes());
    hex_encode(&hasher.finalize())
}

#[allow(dead_code)]
pub(crate) fn write_image_thumbnail(
    source: &Path,
    destination: &Path,
) -> Result<(u32, u32), String> {
    let image = ImageReader::open(source)
        .map_err(|err| {
            format!(
                "Failed to open thumbnail source {}: {err}",
                source.display()
            )
        })?
        .with_guessed_format()
        .map_err(|err| {
            format!(
                "Failed to detect thumbnail source {}: {err}",
                source.display()
            )
        })?
        .decode()
        .map_err(|err| {
            format!(
                "Failed to decode thumbnail source {}: {err}",
                source.display()
            )
        })?;
    let thumbnail = image.thumbnail(THUMBNAIL_MAX_EDGE, THUMBNAIL_MAX_EDGE);
    let rgb = thumbnail.to_rgb8();
    let (width, height) = rgb.dimensions();
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Failed to create thumbnail directory {}: {err}",
                parent.display()
            )
        })?;
    }
    let file = fs::File::create(destination).map_err(|err| {
        format!(
            "Failed to create thumbnail {}: {err}",
            destination.display()
        )
    })?;
    let mut encoder = JpegEncoder::new_with_quality(file, THUMBNAIL_QUALITY);
    encoder
        .encode(&rgb, width, height, image::ColorType::Rgb8.into())
        .map_err(|err| format!("Failed to write thumbnail {}: {err}", destination.display()))?;
    Ok((width, height))
}

fn data_uri_parts(data_uri: &str) -> Result<(&str, &str), String> {
    let Some((header, data)) = data_uri.split_once(',') else {
        return Err("Invalid data URI".to_string());
    };
    let mime_type = header
        .strip_prefix("data:")
        .and_then(|value| value.split_once(';').map(|(mime_type, _)| mime_type))
        .ok_or_else(|| "Invalid data URI header".to_string())?;
    Ok((mime_type, data))
}

fn mime_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

fn first_json_string(value: &serde_json::Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(|entry| entry.as_str())
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
    })
}

fn format_xai_http_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    let payload = serde_json::from_slice::<serde_json::Value>(body).ok();
    let body_text = String::from_utf8_lossy(body).trim().to_string();
    let code = payload
        .as_ref()
        .and_then(|value| first_json_string(value, &["/code", "/error/code"]));
    let detail = payload
        .as_ref()
        .and_then(|value| first_json_string(value, &["/error", "/message", "/error/message"]));
    let summary = [code.as_deref(), detail.as_deref(), Some(body_text.as_str())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = summary.to_ascii_lowercase();

    if normalized.contains("content moderation") || normalized.contains("moderation") {
        return "内容审核未通过，请调整提示词或参考图后重试".to_string();
    }

    if is_xai_billing_error(&normalized) {
        return "xAI 账户额度不足，请充值或调整账单后重试".to_string();
    }

    match status.as_u16() {
        400 => detail
            .or(code)
            .map(|message| format!("xAI 请求失败：{message}"))
            .unwrap_or_else(|| "xAI 请求失败，请检查提示词和参考图".to_string()),
        401 | 403 => "xAI Key 无效或已失效".to_string(),
        408 => "xAI 请求超时，请稍后重试".to_string(),
        429 => "请求过于频繁，请稍后重试".to_string(),
        500..=599 => "xAI 服务暂时不可用，请稍后重试".to_string(),
        _ => detail
            .or(code)
            .map(|message| format!("xAI 请求失败：{message}"))
            .unwrap_or_else(|| format!("xAI 请求失败（HTTP {status}）")),
    }
}

fn is_xai_billing_error(message: &str) -> bool {
    [
        "billing",
        "balance",
        "credit",
        "credits",
        "quota",
        "insufficient",
        "payment",
        "funds",
        "spend",
        "usage limit",
        "额度",
        "余额",
        "费用",
        "充值",
        "账单",
    ]
    .iter()
    .any(|keyword| message.contains(keyword))
}

async fn run_xai_edit_request(
    xai_key: &str,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.x.ai/v1/images/edits")
        .bearer_auth(xai_key)
        .header("Content-Type", "application/json")
        .body(request.to_string())
        .send()
        .await
        .map_err(|err| format!("xAI request failed: {err}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|err| format!("Failed to read xAI response: {err}"))?;
    if !status.is_success() {
        return Err(format_xai_http_error(status, &body));
    }
    serde_json::from_slice(&body).map_err(|err| {
        format!(
            "Failed to parse xAI response: {err}: {}",
            String::from_utf8_lossy(&body)
        )
    })
}

async fn download_url(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|err| format!("Failed to download generated image: {err}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("Failed to read generated image: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "Failed to download generated image: HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    Ok(bytes.to_vec())
}

fn output_image_from_response(response: &serde_json::Value) -> Option<String> {
    response
        .pointer("/url")
        .or_else(|| response.pointer("/image/url"))
        .or_else(|| response.pointer("/image_url"))
        .or_else(|| response.pointer("/data/0/url"))
        .or_else(|| response.pointer("/data/0/image_url"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            response
                .pointer("/base64")
                .or_else(|| response.pointer("/b64_json"))
                .or_else(|| response.pointer("/image/base64"))
                .or_else(|| response.pointer("/data/0/b64_json"))
                .or_else(|| response.pointer("/data/0/base64"))
                .and_then(|value| value.as_str())
                .map(|base64| {
                    if base64.starts_with("data:") {
                        base64.to_string()
                    } else {
                        format!("data:image/png;base64,{base64}")
                    }
                })
        })
}

fn output_images_from_response(response: &serde_json::Value) -> Vec<String> {
    if let Some(items) = response.pointer("/data").and_then(|value| value.as_array()) {
        let images = items
            .iter()
            .filter_map(output_image_from_response)
            .collect::<Vec<_>>();
        if !images.is_empty() {
            return images;
        }
    }
    output_image_from_response(response).into_iter().collect()
}

fn save_generated_image_bytes(
    app: &tauri::AppHandle,
    bytes: &[u8],
    extension: &str,
    source_path: &str,
) -> Result<SavedGeneratedImage, String> {
    let conn = open_db(app)?;
    let dir = configured_generated_content_dir(app, &conn)?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
    allow_asset_directory(&app.asset_protocol_scope(), &dir)?;
    let source_stem = Path::new(source_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let candidate = dir.join(format!("{source_stem}-xai-{}.{}", now_secs(), extension));
    let target = unique_destination_path(&dir, &candidate);
    fs::write(&target, bytes).map_err(|err| format!("Failed to save generated image: {err}"))?;
    Ok(SavedGeneratedImage {
        path: user_path_string(&target),
    })
}

async fn save_generated_image_source(
    app: &tauri::AppHandle,
    image_source: &str,
    source_path: &str,
) -> Result<SavedGeneratedImage, String> {
    if image_source.starts_with("data:") {
        let (mime_type, base64_data) = data_uri_parts(image_source)?;
        let bytes = base64_decode(base64_data)?;
        return save_generated_image_bytes(app, &bytes, mime_extension(mime_type), source_path);
    }

    let bytes = download_url(image_source).await?;
    let extension = image_source
        .split('?')
        .next()
        .and_then(|path| Path::new(path).extension())
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .unwrap_or("png");
    save_generated_image_bytes(app, &bytes, extension, source_path)
}

pub(crate) fn image_extension_matches(format: ImageFormat, extension: &str) -> bool {
    let extension = extension.to_ascii_lowercase();
    match format {
        ImageFormat::Jpeg => matches!(extension.as_str(), "jpg" | "jpeg"),
        ImageFormat::Png => extension == "png",
        ImageFormat::WebP => extension == "webp",
        ImageFormat::Gif => extension == "gif",
        ImageFormat::Bmp => extension == "bmp",
        ImageFormat::Tiff => matches!(extension.as_str(), "tif" | "tiff"),
        _ => false,
    }
}

fn is_supported_video(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "mp4" | "m4v" | "mov" | "webm" | "ogv" | "mkv" | "avi"
            )
        })
        .unwrap_or(false)
}

fn supported_media_type(path: &Path) -> Option<&'static str> {
    if is_supported_image(path) {
        Some("image")
    } else if is_supported_video(path) {
        Some("video")
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn is_windows_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    fs::symlink_metadata(path)
        .map(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

#[cfg(not(target_os = "windows"))]
fn is_windows_reparse_point(_path: &Path) -> bool {
    false
}

fn should_descend_directory(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|metadata| !metadata.file_type().is_symlink())
        .unwrap_or(false)
        && !is_windows_reparse_point(path)
}

pub(crate) fn visit_media(root: &Path, skipped: &mut usize, on_media: &mut impl FnMut(PathBuf)) {
    let mut pending = vec![root.to_path_buf()];
    let mut visited = HashSet::new();

    while let Some(directory) = pending.pop() {
        let Ok(canonical_directory) = fs::canonicalize(&directory) else {
            *skipped += 1;
            continue;
        };
        if !visited.insert(canonical_directory) {
            continue;
        }

        let Ok(entries) = fs::read_dir(&directory) else {
            *skipped += 1;
            continue;
        };

        for entry in entries {
            let Ok(entry) = entry else {
                *skipped += 1;
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                *skipped += 1;
                continue;
            };
            let path = entry.path();
            if file_type.is_dir() {
                if should_descend_directory(&path) {
                    pending.push(path);
                } else {
                    *skipped += 1;
                }
            } else if file_type.is_file() && supported_media_type(&path).is_some() {
                on_media(path);
            } else if file_type.is_symlink() {
                *skipped += 1;
            }
        }
    }
}

pub(crate) fn walk_media(root: &Path, media: &mut Vec<PathBuf>, skipped: &mut usize) {
    visit_media(root, skipped, &mut |path| media.push(path));
}

pub(crate) fn media_size(path: &Path) -> Option<u64> {
    fs::metadata(path).ok().map(|metadata| metadata.len())
}

pub(crate) fn content_hash(path: &Path) -> Result<u64, String> {
    let mut file =
        fs::File::open(path).map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
    let mut hash = 0xcbf29ce484222325u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
        if read == 0 {
            break;
        }
        for byte in &buffer[..read] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(hash)
}

pub(crate) fn files_equal(left: &Path, right: &Path) -> Result<bool, String> {
    if media_size(left) != media_size(right) {
        return Ok(false);
    }
    let mut left_file =
        fs::File::open(left).map_err(|err| format!("Failed to open {}: {err}", left.display()))?;
    let mut right_file = fs::File::open(right)
        .map_err(|err| format!("Failed to open {}: {err}", right.display()))?;
    let mut left_buffer = [0u8; 64 * 1024];
    let mut right_buffer = [0u8; 64 * 1024];
    loop {
        let left_read = left_file
            .read(&mut left_buffer)
            .map_err(|err| format!("Failed to read {}: {err}", left.display()))?;
        let right_read = right_file
            .read(&mut right_buffer)
            .map_err(|err| format!("Failed to read {}: {err}", right.display()))?;
        if left_read != right_read {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
        if left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
    }
}

pub(crate) fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

pub(crate) fn unique_destination_path(destination_dir: &Path, source: &Path) -> PathBuf {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("duplicate");
    let stem = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(file_name);
    let extension = source.extension().and_then(|ext| ext.to_str());
    let mut candidate = destination_dir.join(file_name);
    let mut suffix = 1usize;
    while candidate.exists() {
        let next_name = if let Some(extension) = extension {
            format!("{stem}-{suffix}.{extension}")
        } else {
            format!("{stem}-{suffix}")
        };
        candidate = destination_dir.join(next_name);
        suffix += 1;
    }
    candidate
}

pub(crate) fn move_file(source: &Path, target: &Path) -> Result<(), String> {
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(rename_err) => {
            fs::copy(source, target).map_err(|copy_err| {
                format!(
                    "Failed to move {} to {}: rename failed: {rename_err}; copy failed: {copy_err}",
                    source.display(),
                    target.display()
                )
            })?;
            fs::remove_file(source).map_err(|remove_err| {
                let _ = fs::remove_file(target);
                format!(
                    "Failed to remove original {} after copying to {}: {remove_err}",
                    source.display(),
                    target.display()
                )
            })?;
            Ok(())
        }
    }
}

pub(crate) fn normalize_path(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    fs::canonicalize(trimmed).ok().map(user_path_buf)
}

pub(crate) fn collect_roots(paths: &[String]) -> Vec<PathBuf> {
    let mut roots = paths
        .iter()
        .filter_map(|path| normalize_path(path))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots.into_iter().fold(Vec::new(), |mut kept, root| {
        if !kept.iter().any(|parent: &PathBuf| root.starts_with(parent)) {
            kept.push(root);
        }
        kept
    })
}

fn replace_source_paths(conn: &mut Connection, roots: &[PathBuf]) -> Result<Vec<String>, String> {
    let updated_at = now_secs();
    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed to start source paths transaction: {err}"))?;
    tx.execute("DELETE FROM source_paths", [])
        .map_err(|err| format!("Failed to clear source paths: {err}"))?;

    let mut stored_paths = Vec::with_capacity(roots.len());
    for root in roots {
        let path = user_path_string(root);
        tx.execute(
            "INSERT INTO source_paths (path, created_at) VALUES (?1, ?2)",
            params![path, updated_at],
        )
        .map_err(|err| format!("Failed to save source path: {err}"))?;
        stored_paths.push(user_path_string(root));
    }

    tx.commit()
        .map_err(|err| format!("Failed to commit source paths: {err}"))?;
    Ok(stored_paths)
}

fn user_path_strings(roots: &[PathBuf]) -> Vec<String> {
    roots.iter().map(|root| user_path_string(root)).collect()
}

fn media_path_in_roots(path: &str, roots: &[PathBuf]) -> bool {
    let path = Path::new(path);
    roots.iter().any(|root| path.starts_with(root))
}

fn cleanup_unconfigured_resources(app: tauri::AppHandle) -> Result<(), String> {
    let conn = open_db(&app)?;
    let roots = source_roots_from_conn(&conn)?;
    let mut stmt = conn
        .prepare("SELECT path FROM images")
        .map_err(|err| format!("Failed to prepare resource cleanup query: {err}"))?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("Failed to query resource cleanup paths: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed to collect resource cleanup paths: {err}"))?;
    drop(stmt);

    for path in paths {
        if media_path_in_roots(&path, &roots) {
            continue;
        }
        conn.execute("DELETE FROM images WHERE path = ?1", params![path])
            .map_err(|err| format!("Failed to remove unconfigured resource: {err}"))?;
    }

    conn.execute(
        "
        DELETE FROM image_thumbnails
        WHERE NOT EXISTS (
            SELECT 1 FROM images WHERE images.path = image_thumbnails.image_path
        )
        ",
        [],
    )
    .map_err(|err| format!("Failed to remove unconfigured thumbnail records: {err}"))?;
    Ok(())
}

fn start_resource_cleanup(app: tauri::AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(err) = cleanup_unconfigured_resources(app) {
            eprintln!("Failed to clean unconfigured resources: {err}");
        }
    });
}

#[derive(Clone)]
pub(crate) struct ExistingMediaRecord {
    media_type: String,
    modified: i64,
    size: i64,
}

struct MediaMetadata {
    media_type: String,
    width: u32,
    height: u32,
    modified: i64,
    size: i64,
}

pub(crate) fn load_existing_media_records(
    conn: &Connection,
) -> Result<HashMap<String, ExistingMediaRecord>, String> {
    let mut stmt = conn
        .prepare("SELECT path, media_type, modified, size FROM images")
        .map_err(|err| format!("Failed to prepare existing media query: {err}"))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ExistingMediaRecord {
                    media_type: row.get(1)?,
                    modified: row.get(2)?,
                    size: row.get(3)?,
                },
            ))
        })
        .map_err(|err| format!("Failed to query existing media records: {err}"))?;

    let mut records = HashMap::new();
    for row in rows {
        let (path, record) =
            row.map_err(|err| format!("Failed to read existing media record: {err}"))?;
        records.insert(path, record);
    }
    Ok(records)
}

fn upsert_image_with_metadata(
    conn: &Connection,
    media_path: &Path,
    metadata: &MediaMetadata,
    updated_at: i64,
) -> Result<(), String> {
    conn.execute(
        "
        INSERT INTO images (path, media_type, width, height, modified, size, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(path) DO UPDATE SET
            media_type = excluded.media_type,
            width = excluded.width,
            height = excluded.height,
            modified = excluded.modified,
            size = excluded.size,
            updated_at = excluded.updated_at
        ",
        params![
            user_path_string(media_path),
            metadata.media_type.as_str(),
            metadata.width,
            metadata.height,
            metadata.modified,
            metadata.size,
            updated_at
        ],
    )
    .map_err(|err| format!("Failed to write image record: {err}"))?;

    Ok(())
}

fn touch_image(conn: &Connection, path: &str, updated_at: i64) -> Result<(), String> {
    conn.execute(
        "UPDATE images SET updated_at = ?1 WHERE path = ?2",
        params![updated_at, path],
    )
    .map_err(|err| format!("Failed to touch image record: {err}"))?;
    Ok(())
}

pub(crate) fn upsert_image_incremental(
    conn: &Connection,
    existing_records: &HashMap<String, ExistingMediaRecord>,
    media_path: &Path,
    updated_at: i64,
) -> Result<(), String> {
    let media_type = supported_media_type(media_path)
        .ok_or_else(|| format!("Unsupported media type for {}", media_path.display()))?;
    let (modified, size) = metadata_secs(media_path)?;
    let path = user_path_string(media_path);
    if let Some(existing) = existing_records.get(&path) {
        if existing.media_type == media_type
            && existing.modified == modified
            && existing.size == size
        {
            return touch_image(conn, &path, updated_at);
        }
    }

    let (width, height) = if media_type == "image" {
        image::image_dimensions(media_path).map_err(|err| {
            format!(
                "Failed to read image dimensions for {}: {err}",
                media_path.display()
            )
        })?
    } else {
        video_dimensions(media_path)?
    };

    upsert_image_with_metadata(
        conn,
        media_path,
        &MediaMetadata {
            media_type: media_type.to_string(),
            width,
            height,
            modified,
            size,
        },
        updated_at,
    )
}

pub(crate) fn upsert_image(
    conn: &Connection,
    media_path: &Path,
    updated_at: i64,
) -> Result<(), String> {
    let media_type = supported_media_type(media_path)
        .ok_or_else(|| format!("Unsupported media type for {}", media_path.display()))?;
    let (width, height) = if media_type == "image" {
        image::image_dimensions(media_path).map_err(|err| {
            format!(
                "Failed to read image dimensions for {}: {err}",
                media_path.display()
            )
        })?
    } else {
        video_dimensions(media_path)?
    };
    let (modified, size) = metadata_secs(media_path)?;

    upsert_image_with_metadata(
        conn,
        media_path,
        &MediaMetadata {
            media_type: media_type.to_string(),
            width,
            height,
            modified,
            size,
        },
        updated_at,
    )
}

fn show_window(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(label) {
        if is_gallery_window(label) {
            refresh_asset_scope(app)?;
            let preferences = get_gallery_preferences_from_app(app)?;
            apply_gallery_window_preferences(&window, &preferences)?;
        } else if label == SETTINGS_LABEL {
            window
                .eval("window.location.reload()")
                .map_err(|err| format!("Failed to reload settings window: {err}"))?;
        }
        bring_window_to_front(&window)?;
        return Ok(());
    }

    let (title, view, width, height) = match label {
        SETTINGS_LABEL => ("Gallery Settings", "settings", 760.0, 620.0),
        GALLERY_LABEL => ("Gallery", "gallery", 1240.0, 860.0),
        CAROUSEL_LABEL => ("Carousel", "carousel", 1240.0, 860.0),
        _ => return Err(format!("Unknown window label: {label}")),
    };

    let gallery_preferences = if is_gallery_window(label) {
        refresh_asset_scope(app)?;
        Some(get_gallery_preferences_from_app(app)?)
    } else {
        None
    };
    let url = if let Some(preferences) = &gallery_preferences {
        format!("index.html?view={view}&theme={}", preferences.theme)
    } else {
        format!("index.html?view={view}")
    };
    let background_color = if is_gallery_window(label) {
        gallery_background_color(
            gallery_preferences
                .as_ref()
                .ok_or_else(|| "Missing gallery preferences".to_string())?,
        )
    } else {
        Color(246, 246, 244, 255)
    };

    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(width, height)
        .min_inner_size(520.0, 420.0)
        .resizable(true)
        .maximizable(true)
        .background_color(background_color)
        .center();

    if is_gallery_window(label) {
        let preferences = gallery_preferences
            .as_ref()
            .ok_or_else(|| "Missing gallery preferences".to_string())?;
        #[cfg(target_os = "macos")]
        {
            builder = builder.title_bar_style(TitleBarStyle::Visible);
        }
        builder = builder.theme(Some(if preferences.theme == "black" {
            Theme::Dark
        } else {
            Theme::Light
        }));
    }

    let window = builder
        .build()
        .map_err(|err| format!("Failed to build window: {err}"))?;
    if let Some(preferences) = &gallery_preferences {
        apply_gallery_window_preferences(&window, &preferences)?;
    }
    attach_close_handler(&window);

    bring_window_to_front(&window)?;
    Ok(())
}

fn show_window_from_settings(app: &tauri::AppHandle, label: &str) -> Result<(), String> {
    let settings_window = app.get_webview_window(SETTINGS_LABEL);
    if let Some(window) = &settings_window {
        window
            .hide()
            .map_err(|err| format!("Failed to hide settings window: {err}"))?;
    }

    if let Err(err) = show_window(app, label) {
        if let Some(window) = settings_window {
            let _ = bring_window_to_front(&window);
        }
        return Err(err);
    }

    Ok(())
}

async fn run_window_task(
    app: tauri::AppHandle,
    label: &'static str,
    task: impl FnOnce(tauri::AppHandle) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let app_for_task = app.clone();
    app.run_on_main_thread(move || {
        let result = catch_unwind(AssertUnwindSafe(|| task(app_for_task)))
            .unwrap_or_else(|_| Err(format!("{label} panicked")));
        let _ = sender.send(result);
    })
    .map_err(|err| format!("Failed to schedule {label}: {err}"))?;

    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv()
            .map_err(|err| format!("Failed to receive {label}: {err}"))?
    })
    .await
    .map_err(|err| format!("Failed to wait for {label}: {err}"))?
}

fn attach_close_handler(window: &WebviewWindow) {
    #[cfg(target_os = "windows")]
    {
        let window_for_close = window.clone();
        let prompt_open = Arc::new(Mutex::new(false));
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let app = window_for_close.app_handle().clone();
                let behavior = open_db(&app)
                    .and_then(|conn| windows_close_behavior(&conn))
                    .unwrap_or_else(|_| WINDOWS_CLOSE_BEHAVIOR_ASK.to_string());
                if behavior == WINDOWS_CLOSE_BEHAVIOR_EXIT {
                    app.exit(0);
                    return;
                }
                if behavior == WINDOWS_CLOSE_BEHAVIOR_TRAY {
                    let _ = window_for_close.hide();
                    return;
                }

                let Ok(mut is_prompt_open) = prompt_open.lock() else {
                    return;
                };
                if *is_prompt_open {
                    return;
                }
                *is_prompt_open = true;
                drop(is_prompt_open);

                let window_for_dialog = window_for_close.clone();
                let window_for_action = window_for_close.clone();
                let prompt_open_after_close = Arc::clone(&prompt_open);
                window_for_dialog
                    .dialog()
                    .message("本次关闭方式将作为默认选择保存，之后可在设置中调整。")
                    .parent(&window_for_dialog)
                    .title("关闭 Gallery")
                    .kind(MessageDialogKind::Info)
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "退出应用".to_string(),
                        "保留托盘".to_string(),
                    ))
                    .show(move |should_exit| {
                        if let Ok(mut is_prompt_open) = prompt_open_after_close.lock() {
                            *is_prompt_open = false;
                        }
                        let close_behavior = if should_exit {
                            WINDOWS_CLOSE_BEHAVIOR_EXIT
                        } else {
                            WINDOWS_CLOSE_BEHAVIOR_TRAY
                        };
                        if let Err(err) =
                            persist_windows_close_behavior(&app, close_behavior.to_string())
                        {
                            eprintln!("Failed to persist Windows close behavior: {err}");
                        }
                        let app_for_action = app.clone();
                        let fallback_window = window_for_action.clone();
                        let fallback_app = app.clone();
                        if should_exit {
                            if let Err(err) = app.run_on_main_thread(move || {
                                app_for_action.exit(0);
                            }) {
                                eprintln!("Failed to schedule app exit: {err}");
                                fallback_app.exit(0);
                            }
                        } else {
                            if let Err(err) = app.run_on_main_thread(move || {
                                let _ = window_for_action.hide();
                            }) {
                                eprintln!("Failed to schedule window hide: {err}");
                                let _ = fallback_window.hide();
                            }
                        }
                    });
            }
        });
    }

    #[cfg(not(target_os = "windows"))]
    {
        let window_to_hide = window.clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window_to_hide.hide();
            }
        });
    }
}

fn bring_window_to_front(window: &WebviewWindow) -> Result<(), String> {
    window
        .show()
        .map_err(|err| format!("Failed to show window: {err}"))?;
    window
        .unminimize()
        .map_err(|err| format!("Failed to restore window: {err}"))?;
    window
        .set_focus()
        .map_err(|err| format!("Failed to focus window: {err}"))?;
    Ok(())
}

#[tauri::command]
async fn open_app_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    run_window_task(app, "open window", move |app| show_window(&app, &label)).await
}

#[tauri::command]
async fn open_gallery_from_settings(app: tauri::AppHandle) -> Result<(), String> {
    run_window_task(app, "open gallery from settings", move |app| {
        show_window_from_settings(&app, GALLERY_LABEL)
    })
    .await
}

#[tauri::command]
async fn open_carousel_from_settings(app: tauri::AppHandle) -> Result<(), String> {
    run_window_task(app, "open carousel from settings", move |app| {
        show_window_from_settings(&app, CAROUSEL_LABEL)
    })
    .await
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Result<SettingsState, String> {
    let conn = open_db(&app)?;
    let mut stmt = conn
        .prepare("SELECT path FROM source_paths ORDER BY path COLLATE NOCASE")
        .map_err(|err| format!("Failed to read source paths: {err}"))?;
    let paths = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|err| format!("Failed to query source paths: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed to collect source paths: {err}"))?
        .into_iter()
        .map(|path| user_path_string(&PathBuf::from(path)))
        .collect();
    let image_count = conn
        .query_row("SELECT COUNT(*) FROM images", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("Failed to count images: {err}"))?;
    let preferences = get_gallery_preferences_from_conn(&conn)?;

    Ok(SettingsState {
        platform: current_platform(),
        paths,
        image_count,
        db_path: user_path_string(&db_path(&app)?),
        generated_content_dir: user_path_string(&configured_generated_content_dir(&app, &conn)?),
        thumbnail_enabled: thumbnail_enabled(&conn)?,
        thumbnail_dir: user_path_string(&configured_thumbnail_dir(&app, &conn)?),
        xai_key: read_xai_key_config(&app, &conn)?,
        gallery_mode: normalize_gallery_mode(read_config(&conn, "gallery_mode", "")?, &preferences),
        gallery_has_gap: preferences.has_gap,
        gallery_theme: preferences.theme,
        min_column_width: preferences.min_column_width,
        windows_close_behavior: windows_close_behavior(&conn)?,
    })
}

#[tauri::command]
fn get_gallery_preferences(app: tauri::AppHandle) -> Result<GalleryPreferences, String> {
    get_gallery_preferences_from_app(&app)
}

#[tauri::command]
fn save_gallery_preferences(
    app: tauri::AppHandle,
    mode: String,
    has_gap: bool,
    theme: String,
    min_column_width: u32,
) -> Result<GalleryPreferences, String> {
    let mut preferences = normalize_gallery_preferences(has_gap, theme);
    preferences.min_column_width = min_column_width.clamp(100, 600);
    let conn = open_db(&app)?;
    write_config(
        &conn,
        "gallery_mode",
        &normalize_gallery_mode(mode, &preferences),
    )?;
    write_config(
        &conn,
        "gallery_has_gap",
        if preferences.has_gap { "true" } else { "false" },
    )?;
    write_config(&conn, "gallery_theme", &preferences.theme)?;
    write_config(
        &conn,
        "min_column_width",
        &preferences.min_column_width.to_string(),
    )?;

    if let Some(window) = app.get_webview_window(GALLERY_LABEL) {
        apply_gallery_window_preferences(&window, &preferences)?;
        window
            .eval("window.location.reload()")
            .map_err(|err| format!("Failed to reload gallery window: {err}"))?;
    }
    if let Some(window) = app.get_webview_window(CAROUSEL_LABEL) {
        apply_gallery_window_preferences(&window, &preferences)?;
        window
            .eval("window.location.reload()")
            .map_err(|err| format!("Failed to reload carousel window: {err}"))?;
    }

    Ok(preferences)
}

#[tauri::command]
fn save_source_paths(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<SourcePathsUpdate, String> {
    let roots = collect_roots(&paths);
    let mut conn = open_db(&app)?;
    let existing_paths = user_path_strings(&source_roots_from_conn(&conn)?);
    let incoming_paths = user_path_strings(&roots);
    let changed = incoming_paths != existing_paths;
    let stored_paths = if changed {
        let stored_paths = replace_source_paths(&mut conn, &roots)?;
        refresh_asset_scope_with_conn(&app, &conn)?;
        stored_paths
    } else {
        existing_paths
    };

    Ok(SourcePathsUpdate {
        paths: stored_paths,
        changed,
    })
}

#[tauri::command]
fn save_windows_close_behavior(
    app: tauri::AppHandle,
    close_behavior: String,
) -> Result<String, String> {
    persist_windows_close_behavior(&app, close_behavior)
}

#[tauri::command]
fn save_xai_settings(
    app: tauri::AppHandle,
    xai_key: String,
    generated_content_dir: String,
) -> Result<(), String> {
    let conn = open_db(&app)?;
    let app_dir = canonical_user_path(&app_data_dir(&app)?)?;
    let default_dir = default_generated_content_dir(&app)?;
    let dir = if generated_content_dir.trim().is_empty() {
        default_dir
    } else {
        PathBuf::from(generated_content_dir.trim())
    };
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
    let mut dir = canonical_user_path(&dir)
        .map_err(|err| format!("Failed to canonicalize generated content directory: {err}"))?;
    if dir == app_dir {
        dir = default_generated_content_dir(&app)?;
        fs::create_dir_all(&dir)
            .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
        dir = canonical_user_path(&dir)
            .map_err(|err| format!("Failed to canonicalize generated content directory: {err}"))?;
    }
    write_xai_key_config(&app, &conn, &xai_key)?;
    write_config(&conn, "generated_content_dir", &user_path_string(&dir))?;
    refresh_asset_scope_with_conn(&app, &conn)?;
    Ok(())
}

#[tauri::command]
fn get_xai_key_status(app: tauri::AppHandle) -> Result<XaiKeyStatus, String> {
    let conn = open_db(&app)?;
    let xai_key = read_xai_key_config(&app, &conn)?;
    Ok(XaiKeyStatus {
        configured: !xai_key.trim().is_empty(),
    })
}

#[tauri::command]
fn save_thumbnail_settings(
    app: tauri::AppHandle,
    thumbnail_enabled: bool,
    thumbnail_dir: String,
) -> Result<(), String> {
    let conn = open_db(&app)?;
    if !thumbnail_enabled {
        write_config(&conn, "thumbnail_enabled", "false")?;
        refresh_asset_scope_with_conn(&app, &conn)?;
        return Ok(());
    }

    let dir = if thumbnail_dir.trim().is_empty() {
        default_thumbnail_dir(&app)?
    } else {
        PathBuf::from(thumbnail_dir.trim())
    };
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create thumbnail directory: {err}"))?;
    let dir = canonical_user_path(&dir)
        .map_err(|err| format!("Failed to canonicalize thumbnail directory: {err}"))?;
    write_config(
        &conn,
        "thumbnail_enabled",
        if thumbnail_enabled { "true" } else { "false" },
    )?;
    write_config(&conn, "thumbnail_dir", &user_path_string(&dir))?;
    refresh_asset_scope_with_conn(&app, &conn)?;
    Ok(())
}

#[tauri::command]
fn start_thumbnail_generation(app: tauri::AppHandle) -> Result<(), String> {
    let _ = app;
    Err("缩略图生成已禁用".to_string())
}

#[tauri::command]
fn get_thumbnail_progress(app: tauri::AppHandle) -> Result<ThumbnailProgress, String> {
    app.state::<ThumbnailProgressState>()
        .lock()
        .map(|state| state.clone())
        .map_err(|_| "Failed to read thumbnail progress".to_string())
}

#[tauri::command]
async fn pick_source_folders(window: tauri::Window) -> Result<Vec<String>, String> {
    let folders = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择素材文件夹")
            .blocking_pick_folders()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    let mut paths = folders
        .unwrap_or_default()
        .into_iter()
        .filter_map(|path| path.into_path().ok())
        .filter(|path| path.is_dir())
        .map(|path| user_path_string(&path))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    Ok(paths)
}

#[tauri::command]
async fn pick_duplicate_folder(window: tauri::Window) -> Result<Option<String>, String> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择重复资源保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    folder
        .map(|path| {
            path.into_path()
                .map(|path| user_path_string(&path))
                .map_err(|err| format!("Failed to resolve duplicate folder: {err}"))
        })
        .transpose()
}

#[tauri::command]
async fn pick_generated_content_folder(window: tauri::Window) -> Result<Option<String>, String> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择生成内容保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    folder
        .map(|path| {
            path.into_path()
                .map(|path| user_path_string(&path))
                .map_err(|err| format!("Failed to resolve generated content folder: {err}"))
        })
        .transpose()
}

#[tauri::command]
async fn pick_thumbnail_folder(window: tauri::Window) -> Result<Option<String>, String> {
    let folder = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择缩略图保存位置")
            .blocking_pick_folder()
    })
    .await
    .map_err(|err| format!("Failed to open folder picker: {err}"))?;

    folder
        .map(|path| {
            path.into_path()
                .map(|path| user_path_string(&path))
                .map_err(|err| format!("Failed to resolve thumbnail folder: {err}"))
        })
        .transpose()
}

#[tauri::command]
async fn pick_xai_reference_images(window: tauri::Window) -> Result<Vec<PickedImage>, String> {
    let files = tauri::async_runtime::spawn_blocking(move || {
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title("选择参考图片")
            .blocking_pick_files()
    })
    .await
    .map_err(|err| format!("Failed to open image picker: {err}"))?;

    files
        .unwrap_or_default()
        .into_iter()
        .take(3)
        .filter_map(|path| path.into_path().ok())
        .filter(|path| path.is_file() && is_supported_image(path))
        .map(|path| {
            let path = user_path_buf(path);
            let data_uri = read_image_data_uri(user_path_string(&path))?;
            Ok(PickedImage {
                path: user_path_string(&path),
                data_uri,
            })
        })
        .collect()
}

#[tauri::command]
async fn scan_library(app: tauri::AppHandle, paths: Vec<String>) -> Result<ScanSummary, String> {
    tauri::async_runtime::spawn_blocking(move || scanner::scan_library(app, paths))
        .await
        .map_err(|err| format!("Failed to run scan: {err}"))?
}

#[tauri::command]
async fn deduplicate_resources(
    app: tauri::AppHandle,
    paths: Vec<String>,
    destination_path: String,
) -> Result<DedupeSummary, String> {
    tauri::async_runtime::spawn_blocking(move || {
        scanner::deduplicate_resources(app, paths, destination_path)
    })
    .await
    .map_err(|err| format!("Failed to run duplicate detection: {err}"))?
}

#[tauri::command]
async fn repair_image_extensions(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<ExtensionRepairSummary, String> {
    tauri::async_runtime::spawn_blocking(move || scanner::repair_image_extensions(app, paths))
        .await
        .map_err(|err| format!("Failed to run extension repair: {err}"))?
}

#[tauri::command]
fn read_image_data_uri(path: String) -> Result<String, String> {
    let path = normalize_path(&path).ok_or_else(|| "Invalid image path".to_string())?;
    if !path.is_file() || !is_supported_image(&path) {
        return Err("Unsupported image".to_string());
    }
    let format = image_format(&path).ok_or_else(|| "Unsupported image".to_string())?;
    let mime_type = image_mime_type(format).ok_or_else(|| "Unsupported image".to_string())?;
    let bytes = fs::read(&path).map_err(|err| format!("Failed to read image: {err}"))?;
    Ok(format!("data:{mime_type};base64,{}", base64_encode(&bytes)))
}

#[tauri::command]
fn save_generated_image(
    app: tauri::AppHandle,
    data_uri: String,
    source_path: String,
) -> Result<SavedGeneratedImage, String> {
    let conn = open_db(&app)?;
    let dir = configured_generated_content_dir(&app, &conn)?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
    allow_asset_directory(&app.asset_protocol_scope(), &dir)?;
    let source_stem = Path::new(&source_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let (mime_type, base64_data) = data_uri_parts(&data_uri)?;
    let bytes = base64_decode(base64_data)?;
    let extension = mime_extension(mime_type);
    let candidate = dir.join(format!("{source_stem}-xai-{}.{}", now_secs(), extension));
    let target = unique_destination_path(&dir, &candidate);
    fs::write(&target, bytes).map_err(|err| format!("Failed to save generated image: {err}"))?;
    Ok(SavedGeneratedImage {
        path: user_path_string(&target),
    })
}

#[tauri::command]
fn archive_xai_edit(app: tauri::AppHandle, entry: XaiEditArchiveEntry) -> Result<(), String> {
    let dir = app_data_dir(&app)?.join("xai-edit-archives");
    fs::create_dir_all(&dir).map_err(|err| format!("Failed to create archive directory: {err}"))?;
    let source_stem = Path::new(&entry.source_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let archive_path = unique_destination_path(
        &dir,
        &dir.join(format!("{source_stem}-{}.json", entry.created_at)),
    );
    let json = serde_json::to_vec_pretty(&entry)
        .map_err(|err| format!("Failed to serialize archive: {err}"))?;
    fs::write(archive_path, json).map_err(|err| format!("Failed to write archive: {err}"))?;
    Ok(())
}

#[tauri::command]
async fn edit_image_with_xai(
    app: tauri::AppHandle,
    source_paths: Vec<String>,
    source_data_uris: Vec<String>,
    prompt: String,
    aspect_ratio: Option<String>,
    resolution: Option<String>,
    image_count: Option<u8>,
) -> Result<XaiEditResult, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("提示词不能为空".to_string());
    }
    if source_paths.is_empty() || source_data_uris.is_empty() {
        return Err("参考图不能为空".to_string());
    }
    if source_paths.len() != source_data_uris.len() {
        return Err("参考图数据不完整".to_string());
    }

    let conn = open_db(&app)?;
    let xai_key = read_xai_key_config(&app, &conn)?;
    if xai_key.trim().is_empty() {
        return Err("xAI Key 未设置".to_string());
    }

    let source_path = source_paths
        .first()
        .cloned()
        .ok_or_else(|| "参考图不能为空".to_string())?;
    let source_label = Path::new(&source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    let image_inputs = source_data_uris
        .iter()
        .take(3)
        .map(|uri| {
            serde_json::json!({
                "type": "image_url",
                "url": uri,
            })
        })
        .collect::<Vec<_>>();
    let mut request = serde_json::json!({
        "model": "grok-imagine-image-quality",
        "prompt": prompt,
    });
    let image_count = image_count.unwrap_or(1).clamp(1, 4);
    request["n"] = serde_json::Value::Number(serde_json::Number::from(image_count));
    let aspect_ratio = normalize_optional_text(aspect_ratio).filter(|value| value != "auto");
    if let Some(ratio) = &aspect_ratio {
        validate_xai_image_aspect_ratio(ratio)?;
        request["aspect_ratio"] = serde_json::Value::String(ratio.clone());
    }
    let resolution = normalize_optional_text(resolution);
    if let Some(size) = &resolution {
        validate_xai_image_resolution(size)?;
        request["resolution"] = serde_json::Value::String(size.clone());
    }
    if image_inputs.len() == 1 {
        request["image"] = image_inputs[0].clone();
    } else {
        request["images"] = serde_json::Value::Array(image_inputs);
    }
    let response = run_xai_edit_request(xai_key.trim(), &request).await?;
    let image_sources = output_images_from_response(&response);
    if image_sources.is_empty() {
        return Err("未返回图片".to_string());
    }
    let mut output_paths = Vec::with_capacity(image_sources.len());
    for image_source in image_sources {
        let saved = save_generated_image_source(&app, &image_source, &source_path).await?;
        output_paths.push(saved.path);
    }
    let output_path = output_paths
        .first()
        .cloned()
        .ok_or_else(|| "未返回图片".to_string())?;
    let created_at = now_secs();
    archive_xai_edit(
        app,
        XaiEditArchiveEntry {
            source_path,
            source_label,
            prompt: prompt.to_string(),
            aspect_ratio,
            resolution,
            image_count,
            output_path: Some(output_path.clone()),
            output_paths: output_paths.clone(),
            response: response.clone(),
            created_at,
        },
    )?;
    Ok(XaiEditResult {
        path: output_path,
        paths: output_paths,
        response,
    })
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn validate_xai_image_aspect_ratio(aspect_ratio: &str) -> Result<(), String> {
    if matches!(
        aspect_ratio,
        "1:1"
            | "16:9"
            | "9:16"
            | "4:3"
            | "3:4"
            | "3:2"
            | "2:3"
            | "2:1"
            | "1:2"
            | "20:9"
            | "9:20"
            | "auto"
    ) {
        Ok(())
    } else {
        Err("不支持的图片比例".to_string())
    }
}

fn validate_xai_image_resolution(resolution: &str) -> Result<(), String> {
    if matches!(resolution, "1k" | "2k") {
        Ok(())
    } else {
        Err("不支持的图片分辨率".to_string())
    }
}

#[tauri::command]
fn load_editor_session(app: tauri::AppHandle) -> Result<EditorSessionState, String> {
    let session_id = current_editor_session_id(&app)?;
    let session_dir = editor_sessions_dir(&app)?.join(&session_id);
    if !session_dir.exists() {
        return Ok(EditorSessionState {
            session_id,
            messages: Vec::new(),
        });
    }

    let mut segment_paths = fs::read_dir(&session_dir)
        .map_err(|err| format!("Failed to read editor session directory: {err}"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("segment-") && name.ends_with(".json"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    segment_paths.sort();

    let mut messages = Vec::new();
    for path in segment_paths {
        let bytes = fs::read(&path)
            .map_err(|err| format!("Failed to read editor session segment: {err}"))?;
        let segment = serde_json::from_slice::<EditorSessionSegment>(&bytes)
            .map_err(|err| format!("Failed to parse editor session segment: {err}"))?;
        if segment.session_id == session_id {
            messages.extend(segment.messages);
        }
    }

    Ok(EditorSessionState {
        session_id,
        messages,
    })
}

#[tauri::command]
fn save_editor_session(
    app: tauri::AppHandle,
    session_id: String,
    messages: Vec<EditorSessionMessage>,
) -> Result<(), String> {
    validate_editor_session_id(&session_id)?;
    write_editor_session_meta(&app, &session_id)?;
    let session_dir = editor_sessions_dir(&app)?.join(&session_id);
    fs::create_dir_all(&session_dir)
        .map_err(|err| format!("Failed to create editor session directory: {err}"))?;

    for entry in fs::read_dir(&session_dir)
        .map_err(|err| format!("Failed to inspect editor session directory: {err}"))?
        .flatten()
    {
        let path = entry.path();
        let is_segment = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with("segment-") && name.ends_with(".json"))
            .unwrap_or(false);
        if is_segment {
            fs::remove_file(path)
                .map_err(|err| format!("Failed to remove old editor session segment: {err}"))?;
        }
    }

    let now = now_secs();
    let mut segments: Vec<Vec<EditorSessionMessage>> = Vec::new();
    let mut current_segment: Vec<EditorSessionMessage> = Vec::new();
    let mut turn_count = 0usize;

    for message in messages {
        if message.role == "user" && turn_count >= EDITOR_SESSION_SEGMENT_TURNS {
            segments.push(current_segment);
            current_segment = Vec::new();
            turn_count = 0;
        }
        if message.role == "user" {
            turn_count += 1;
        }
        current_segment.push(message);
    }

    if !current_segment.is_empty() {
        segments.push(current_segment);
    }

    for (segment_index, segment_messages) in segments.into_iter().enumerate() {
        let segment = EditorSessionSegment {
            session_id: session_id.clone(),
            segment_index,
            messages: segment_messages,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_vec_pretty(&segment)
            .map_err(|err| format!("Failed to serialize editor session segment: {err}"))?;
        let segment_path = session_dir.join(format!("segment-{segment_index:05}.json"));
        fs::write(segment_path, json)
            .map_err(|err| format!("Failed to write editor session segment: {err}"))?;
    }

    Ok(())
}

#[tauri::command]
fn list_images(
    app: tauri::AppHandle,
    cursor: Option<ImageCursor>,
    limit: i64,
) -> Result<ImagePage, String> {
    gallery::list_images(app, cursor, limit)
}

#[tauri::command]
fn list_random_images(app: tauri::AppHandle, limit: i64) -> Result<Vec<ImageRecord>, String> {
    gallery::list_random_images(app, limit)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if !claim_single_instance() {
        return;
    }

    tauri::Builder::default()
        .manage(Arc::new(Mutex::new(initial_thumbnail_progress())) as ThumbnailProgressState)
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            if let Err(err) = refresh_asset_scope(&app.handle().clone()) {
                eprintln!("Failed to refresh asset scope: {err}");
            }
            start_resource_cleanup(app.handle().clone());

            let settings = MenuItem::with_id(app, SETTINGS_MENU_ID, "设置", true, None::<&str>)?;
            let gallery = MenuItem::with_id(app, GALLERY_MENU_ID, "瀑布流", true, None::<&str>)?;
            let carousel = MenuItem::with_id(app, CAROUSEL_MENU_ID, "走马灯", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, QUIT_MENU_ID, "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings, &gallery, &carousel, &quit])?;
            let mut tray_builder = TrayIconBuilder::with_id("gallery")
                .tooltip("Gallery")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == QUIT_MENU_ID {
                        app.exit(0);
                        return;
                    }

                    let label = match id {
                        SETTINGS_MENU_ID => Some(SETTINGS_LABEL),
                        GALLERY_MENU_ID => Some(GALLERY_LABEL),
                        CAROUSEL_MENU_ID => Some(CAROUSEL_LABEL),
                        _ => None,
                    };
                    if let Some(label) = label {
                        let _ = show_window(app, label);
                    }
                });

            if let Some(icon) = tray_icon() {
                tray_builder = tray_builder.icon(icon).icon_as_template(false);
            } else if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }
            tray_builder.build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            open_app_window,
            open_gallery_from_settings,
            open_carousel_from_settings,
            get_settings,
            get_gallery_preferences,
            save_gallery_preferences,
            save_source_paths,
            save_windows_close_behavior,
            save_xai_settings,
            get_xai_key_status,
            save_thumbnail_settings,
            start_thumbnail_generation,
            get_thumbnail_progress,
            pick_source_folders,
            pick_duplicate_folder,
            pick_generated_content_folder,
            pick_thumbnail_folder,
            pick_xai_reference_images,
            scan_library,
            deduplicate_resources,
            repair_image_extensions,
            read_image_data_uri,
            save_generated_image,
            archive_xai_edit,
            edit_image_with_xai,
            load_editor_session,
            save_editor_session,
            list_images,
            list_random_images
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
