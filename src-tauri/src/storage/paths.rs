//! 应用私有存储路径解析。
//! 负责定位并创建 Tauri app data 目录，以及派生 gallery.sqlite3 文件路径。
//! 不打开数据库，也不管理具体业务子目录。

use std::{fs, path::PathBuf};
use tauri::Manager;

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
