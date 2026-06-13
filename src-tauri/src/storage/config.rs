//! 应用配置和偏好设置服务。
//! 管理 app_config 中的路径、图库偏好、xAI key 加密和 Windows 启动项设置。
//! 不直接处理前端命令展示逻辑，命令层负责组装 SettingsState。

use crate::{
    shared::{
        encoding::{base64_decode, base64_encode},
        models::{GalleryPreferences, WindowsStartupSettings},
        path_utils::{canonical_user_path, user_path_buf, user_path_string},
    },
    storage::{
        db::{open_db, read_config, write_config},
        paths::app_data_dir,
    },
};
use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm,
};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{env, path::PathBuf};

const ENCRYPTED_XAI_KEY_PREFIX: &str = "enc:v1:";
pub(crate) const WINDOWS_CLOSE_BEHAVIOR_CONFIG_KEY: &str = "windows_close_behavior";
pub(crate) const WINDOWS_CLOSE_BEHAVIOR_ASK: &str = "ask";
pub(crate) const WINDOWS_CLOSE_BEHAVIOR_EXIT: &str = "exit";
pub(crate) const WINDOWS_CLOSE_BEHAVIOR_TRAY: &str = "tray";
const WINDOWS_STARTUP_ENABLED_CONFIG_KEY: &str = "windows_startup_enabled";
const WINDOWS_STARTUP_DESKTOP_BACKGROUND_CONFIG_KEY: &str = "windows_startup_desktop_background";
#[cfg(target_os = "windows")]
const WINDOWS_STARTUP_DESKTOP_BACKGROUND_ARG: &str = "--desktop-background-on-startup";

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

pub(crate) fn normalize_windows_close_behavior(value: String) -> String {
    match value.as_str() {
        WINDOWS_CLOSE_BEHAVIOR_EXIT | WINDOWS_CLOSE_BEHAVIOR_TRAY | WINDOWS_CLOSE_BEHAVIOR_ASK => {
            value
        }
        _ => WINDOWS_CLOSE_BEHAVIOR_ASK.to_string(),
    }
}

pub(crate) fn windows_close_behavior(conn: &Connection) -> Result<String, String> {
    Ok(normalize_windows_close_behavior(read_config(
        conn,
        WINDOWS_CLOSE_BEHAVIOR_CONFIG_KEY,
        WINDOWS_CLOSE_BEHAVIOR_ASK,
    )?))
}

pub(crate) fn persist_windows_close_behavior(
    app: &tauri::AppHandle,
    close_behavior: String,
) -> Result<String, String> {
    let conn = open_db(app)?;
    let close_behavior = normalize_windows_close_behavior(close_behavior);
    write_config(&conn, WINDOWS_CLOSE_BEHAVIOR_CONFIG_KEY, &close_behavior)?;
    Ok(close_behavior)
}

fn config_bool(conn: &Connection, key: &str) -> Result<bool, String> {
    Ok(read_config(conn, key, "false")? == "true")
}

pub(crate) fn windows_startup_settings(
    conn: &Connection,
) -> Result<WindowsStartupSettings, String> {
    let startup_enabled = config_bool(conn, WINDOWS_STARTUP_ENABLED_CONFIG_KEY)?;
    Ok(WindowsStartupSettings {
        startup_enabled,
        startup_desktop_background: startup_enabled
            && config_bool(conn, WINDOWS_STARTUP_DESKTOP_BACKGROUND_CONFIG_KEY)?,
    })
}

pub(crate) fn persist_windows_startup_settings(
    app: &tauri::AppHandle,
    startup_enabled: bool,
    startup_desktop_background: bool,
) -> Result<WindowsStartupSettings, String> {
    let conn = open_db(app)?;
    let settings = WindowsStartupSettings {
        startup_enabled,
        startup_desktop_background: startup_enabled && startup_desktop_background,
    };
    write_config(
        &conn,
        WINDOWS_STARTUP_ENABLED_CONFIG_KEY,
        if settings.startup_enabled {
            "true"
        } else {
            "false"
        },
    )?;
    write_config(
        &conn,
        WINDOWS_STARTUP_DESKTOP_BACKGROUND_CONFIG_KEY,
        if settings.startup_desktop_background {
            "true"
        } else {
            "false"
        },
    )?;
    apply_windows_startup_registry(
        settings.startup_enabled,
        settings.startup_desktop_background,
    )?;
    Ok(settings)
}

pub(crate) fn sync_windows_startup_registry_from_config(app: &tauri::AppHandle) {
    let result = (|| -> Result<(), String> {
        let conn = open_db(app)?;
        let settings = windows_startup_settings(&conn)?;
        apply_windows_startup_registry(
            settings.startup_enabled,
            settings.startup_desktop_background,
        )
    })();
    if let Err(err) = result {
        eprintln!("Failed to sync Windows startup settings: {err}");
    }
}

#[cfg(target_os = "windows")]
fn apply_windows_startup_registry(
    startup_enabled: bool,
    startup_desktop_background: bool,
) -> Result<(), String> {
    #[link(name = "advapi32")]
    extern "system" {
        fn RegCreateKeyExW(
            key: isize,
            sub_key: *const u16,
            reserved: u32,
            class: *mut u16,
            options: u32,
            desired: u32,
            security_attributes: *mut std::ffi::c_void,
            result_key: *mut isize,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: isize,
            value_name: *const u16,
            reserved: u32,
            value_type: u32,
            data: *const u8,
            data_size: u32,
        ) -> i32;
        fn RegDeleteValueW(key: isize, value_name: *const u16) -> i32;
        fn RegCloseKey(key: isize) -> i32;
    }

    const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
    const ERROR_SUCCESS: i32 = 0;
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const KEY_SET_VALUE: u32 = 0x0002;
    const REG_OPTION_NON_VOLATILE: u32 = 0;
    const REG_SZ: u32 = 1;
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const RUN_VALUE: &str = "Gallery";

    let mut run_key = 0isize;
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide_null(RUN_KEY).as_ptr(),
            0,
            std::ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            std::ptr::null_mut(),
            &mut run_key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Failed to open Windows startup registry key: {status}"
        ));
    }

    let value_name = wide_null(RUN_VALUE);
    let result = if startup_enabled {
        let executable = env::current_exe()
            .map_err(|err| format!("Failed to resolve current executable: {err}"))?;
        let mut command = format!("\"{}\"", user_path_string(&executable));
        if startup_desktop_background {
            command.push(' ');
            command.push_str(WINDOWS_STARTUP_DESKTOP_BACKGROUND_ARG);
        }
        let command = wide_null(&command);
        let status = unsafe {
            RegSetValueExW(
                run_key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast::<u8>(),
                u32::try_from(command.len() * std::mem::size_of::<u16>()).unwrap_or(u32::MAX),
            )
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(format!(
                "Failed to write Windows startup registry value: {status}"
            ))
        }
    } else {
        let status = unsafe { RegDeleteValueW(run_key, value_name.as_ptr()) };
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(format!(
                "Failed to remove Windows startup registry value: {status}"
            ))
        }
    };

    let close_status = unsafe { RegCloseKey(run_key) };
    if close_status != ERROR_SUCCESS {
        return Err(format!(
            "Failed to close Windows startup registry key: {close_status}"
        ));
    }
    result
}

#[cfg(not(target_os = "windows"))]
fn apply_windows_startup_registry(
    _startup_enabled: bool,
    _startup_desktop_background: bool,
) -> Result<(), String> {
    Ok(())
}

pub(crate) fn launched_from_desktop_background_startup() -> bool {
    #[cfg(target_os = "windows")]
    {
        env::args().any(|arg| arg == WINDOWS_STARTUP_DESKTOP_BACKGROUND_ARG)
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
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

pub(crate) fn read_xai_key_config(
    app: &tauri::AppHandle,
    conn: &Connection,
) -> Result<String, String> {
    let stored = read_config(conn, "xai_key", "")?;
    if stored.is_empty() {
        Ok(stored)
    } else {
        decrypt_xai_key(app, &stored)
    }
}

pub(crate) fn write_xai_key_config(
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

pub(crate) fn normalize_gallery_preferences(has_gap: bool, theme: String) -> GalleryPreferences {
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

pub(crate) fn get_gallery_preferences_from_conn(
    conn: &Connection,
) -> Result<GalleryPreferences, String> {
    let has_gap = read_config(conn, "gallery_has_gap", "false")? == "true";
    let theme = read_config(conn, "gallery_theme", "white")?;
    let mut preferences = normalize_gallery_preferences(has_gap, theme);
    preferences.min_column_width = read_config(conn, "min_column_width", "280")?
        .parse::<u32>()
        .unwrap_or(280)
        .clamp(100, 600);
    Ok(preferences)
}

pub(crate) fn get_gallery_preferences_from_app(
    app: &tauri::AppHandle,
) -> Result<GalleryPreferences, String> {
    let conn = open_db(app)?;
    get_gallery_preferences_from_conn(&conn)
}

pub(crate) fn normalize_gallery_mode(mode: String, preferences: &GalleryPreferences) -> String {
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

pub(crate) fn current_platform() -> String {
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
