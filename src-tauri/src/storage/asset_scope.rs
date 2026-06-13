//! Tauri asset 协议权限管理。
//! 根据素材源、应用数据目录、生成内容目录和缩略图目录刷新可访问范围。
//! 不负责扫描或保存资源，只维护前端能通过 asset:// 读取哪些本地路径。

use crate::{
    shared::path_utils::user_path_buf,
    storage::{
        config::{configured_generated_content_dir, configured_thumbnail_dir},
        db::open_db,
        paths::app_data_dir,
    },
};
use rusqlite::Connection;
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::Manager;

pub(crate) fn source_roots_from_conn(conn: &Connection) -> Result<Vec<PathBuf>, String> {
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

pub(crate) fn allow_asset_directory(
    scope: &tauri::scope::fs::Scope,
    directory: &Path,
) -> Result<(), String> {
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

pub(crate) fn refresh_asset_scope(app: &tauri::AppHandle) -> Result<(), String> {
    let conn = open_db(app)?;
    refresh_asset_scope_with_conn(app, &conn)
}
