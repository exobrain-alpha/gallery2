//! 素材源路径配置维护。
//! 负责规范化用户选择的素材目录、写入 source_paths 表，并清理已不属于配置目录的资源记录。
//! 不做媒体文件扫描；扫描流程由 library::scanner 根据这里保存的路径执行。

use crate::{
    shared::{
        path_utils::{normalize_path, user_path_string},
        time::now_secs,
    },
    storage::{asset_scope::source_roots_from_conn, db::open_db},
};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

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

pub(crate) fn replace_source_paths(
    conn: &mut Connection,
    roots: &[PathBuf],
) -> Result<Vec<String>, String> {
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

pub(crate) fn user_path_strings(roots: &[PathBuf]) -> Vec<String> {
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

pub(crate) fn start_resource_cleanup(app: tauri::AppHandle) {
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(err) = cleanup_unconfigured_resources(app) {
            eprintln!("Failed to clean unconfigured resources: {err}");
        }
    });
}
