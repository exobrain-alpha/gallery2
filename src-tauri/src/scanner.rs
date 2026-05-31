use crate::models::{DedupeSummary, ExtensionRepairSummary, ScanSummary, ThumbnailProgress};
use crate::{
    collect_roots, configured_thumbnail_dir, content_hash, files_equal, image_extension_matches,
    image_format, image_format_extension, is_supported_image, load_existing_media_records,
    media_size, move_file, normalize_path, now_nanos, now_secs, open_db, paths_overlap,
    thumbnail_cache_key, unique_destination_path, upsert_image, upsert_image_incremental,
    visit_media, walk_media, write_image_thumbnail, CAROUSEL_LABEL, DEDUPE_MAX_FILE_SIZE,
    DESKTOP_BACKGROUND_LABEL, GALLERY_LABEL,
};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tauri::Manager;

#[allow(dead_code)]
struct ThumbnailSource {
    path: String,
    modified: i64,
    size: i64,
}

pub(crate) fn scan_library(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<ScanSummary, String> {
    let roots = collect_roots(&paths);
    let mut conn = open_db(&app)?;
    let existing_records = load_existing_media_records(&conn)?;
    let updated_at = now_nanos();
    let tx = conn
        .transaction()
        .map_err(|err| format!("Failed to start scan transaction: {err}"))?;

    let mut indexed = 0usize;
    let mut skipped = 0usize;
    for root in &roots {
        let mut root_skipped = 0usize;
        let mut failed = 0usize;
        visit_media(
            root,
            &mut root_skipped,
            &mut |media_path| match upsert_image_incremental(
                &tx,
                &existing_records,
                &media_path,
                updated_at,
            ) {
                Ok(()) => {
                    indexed += 1;
                }
                Err(_) => failed += 1,
            },
        );
        skipped += root_skipped + failed;
    }

    let removed = tx
        .execute(
            "DELETE FROM images WHERE updated_at != ?1",
            params![updated_at],
        )
        .map_err(|err| format!("Failed to remove stale images: {err}"))?;
    tx.execute(
        "
        DELETE FROM image_thumbnails
        WHERE NOT EXISTS (
            SELECT 1 FROM images WHERE images.path = image_thumbnails.image_path
        )
        ",
        [],
    )
    .map_err(|err| format!("Failed to remove stale thumbnail records: {err}"))?;
    let total = tx
        .query_row("SELECT COUNT(*) FROM images", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("Failed to count images: {err}"))?;
    tx.commit()
        .map_err(|err| format!("Failed to commit scan results: {err}"))?;
    reload_gallery_window(&app);

    Ok(ScanSummary {
        indexed,
        skipped,
        removed,
        total,
    })
}

#[allow(dead_code)]
pub(crate) fn generate_thumbnails(app: tauri::AppHandle, progress: Arc<Mutex<ThumbnailProgress>>) {
    set_thumbnail_progress(&progress, |state| {
        state.running = true;
        state.stage = "generating".to_string();
        state.processed = 0;
        state.total = 0;
        state.generated = 0;
        state.skipped = 0;
        state.message = "生成缩略图中...".to_string();
        state.error.clear();
    });

    let result = (|| -> Result<(), String> {
        let conn = open_db(&app)?;
        let thumbnail_dir = configured_thumbnail_dir(&app, &conn)?;
        fs::create_dir_all(&thumbnail_dir)
            .map_err(|err| format!("Failed to create thumbnail directory: {err}"))?;
        app.asset_protocol_scope()
            .allow_directory(&thumbnail_dir, true)
            .map_err(|err| format!("Failed to allow thumbnail directory: {err}"))?;
        let sources = thumbnail_sources(&conn)?;
        set_thumbnail_progress(&progress, |state| {
            state.stage = "generating".to_string();
            state.processed = 0;
            state.total = sources.len();
            state.generated = 0;
            state.skipped = 0;
            state.message = format!("生成缩略图 0/{}", sources.len());
        });

        for source in sources {
            match ensure_thumbnail(&conn, &thumbnail_dir, &source) {
                Ok(true) => set_thumbnail_progress(&progress, |state| {
                    state.processed += 1;
                    state.generated += 1;
                    state.message = format!("生成缩略图 {}/{}", state.processed, state.total);
                }),
                Ok(false) => set_thumbnail_progress(&progress, |state| {
                    state.processed += 1;
                    state.skipped += 1;
                    state.message = format!("复用缩略图 {}/{}", state.processed, state.total);
                }),
                Err(_) => set_thumbnail_progress(&progress, |state| {
                    state.processed += 1;
                    state.skipped += 1;
                    state.message = format!("跳过失败资源 {}/{}", state.processed, state.total);
                }),
            }
        }

        set_thumbnail_progress(&progress, |state| {
            state.running = false;
            state.stage = "done".to_string();
            state.message = format!(
                "缩略图完成：生成 {} 个，复用/跳过 {} 个",
                state.generated, state.skipped
            );
        });
        Ok(())
    })();

    if let Err(error) = result {
        set_thumbnail_progress(&progress, |state| {
            state.running = false;
            state.stage = "error".to_string();
            state.error = error.clone();
            state.message = error;
        });
    }
}

fn reload_gallery_window(app: &tauri::AppHandle) {
    for label in [GALLERY_LABEL, CAROUSEL_LABEL, DESKTOP_BACKGROUND_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.eval("window.dispatchEvent(new CustomEvent('gallery:reload'))");
        }
    }
}

#[allow(dead_code)]
fn set_thumbnail_progress(
    progress: &Arc<Mutex<ThumbnailProgress>>,
    update: impl FnOnce(&mut ThumbnailProgress),
) {
    if let Ok(mut state) = progress.lock() {
        update(&mut state);
    }
}

#[allow(dead_code)]
fn thumbnail_sources(conn: &Connection) -> Result<Vec<ThumbnailSource>, String> {
    let mut stmt = conn
        .prepare(
            "
            SELECT path, modified, size
            FROM images
            WHERE media_type = 'image'
            ORDER BY modified DESC, path COLLATE NOCASE ASC
            ",
        )
        .map_err(|err| format!("Failed to prepare thumbnail source query: {err}"))?;
    let sources = stmt
        .query_map([], |row| {
            Ok(ThumbnailSource {
                path: row.get(0)?,
                modified: row.get(1)?,
                size: row.get(2)?,
            })
        })
        .map_err(|err| format!("Failed to query thumbnail sources: {err}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| format!("Failed to collect thumbnail sources: {err}"))?;
    Ok(sources)
}

#[allow(dead_code)]
fn ensure_thumbnail(
    conn: &Connection,
    thumbnail_dir: &Path,
    source: &ThumbnailSource,
) -> Result<bool, String> {
    let existing = conn
        .query_row(
            "
            SELECT thumb_path
            FROM image_thumbnails
            WHERE image_path = ?1 AND source_modified = ?2 AND source_size = ?3
            ",
            params![source.path, source.modified, source.size],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| format!("Failed to query thumbnail record: {err}"))?;
    if let Some(path) = existing {
        let path = PathBuf::from(path);
        if path.starts_with(thumbnail_dir) && path.is_file() {
            return Ok(false);
        }
    }

    let key = thumbnail_cache_key(&source.path, source.modified, source.size);
    let shard = key.get(0..2).unwrap_or("00");
    let thumb_path = thumbnail_dir.join(shard).join(format!("{key}.jpg"));
    let old_thumb_path = conn
        .query_row(
            "SELECT thumb_path FROM image_thumbnails WHERE image_path = ?1",
            params![source.path],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|err| format!("Failed to query old thumbnail record: {err}"))?;
    let (width, height) = write_image_thumbnail(Path::new(&source.path), &thumb_path)?;
    let thumb_path = thumb_path.to_string_lossy().into_owned();
    conn.execute(
        "
        INSERT INTO image_thumbnails (
            image_path, thumb_path, source_modified, source_size, width, height, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(image_path) DO UPDATE SET
            thumb_path = excluded.thumb_path,
            source_modified = excluded.source_modified,
            source_size = excluded.source_size,
            width = excluded.width,
            height = excluded.height,
            updated_at = excluded.updated_at
        ",
        params![
            source.path,
            thumb_path,
            source.modified,
            source.size,
            width,
            height,
            now_secs()
        ],
    )
    .map_err(|err| format!("Failed to save thumbnail record: {err}"))?;
    if let Some(old_path) = old_thumb_path.filter(|old_path| old_path != &thumb_path) {
        let _ = fs::remove_file(old_path);
    }
    Ok(true)
}

pub(crate) fn deduplicate_resources(
    app: tauri::AppHandle,
    paths: Vec<String>,
    destination_path: String,
) -> Result<DedupeSummary, String> {
    let roots = collect_roots(&paths);
    let destination = normalize_path(&destination_path)
        .filter(|path| path.is_dir())
        .ok_or_else(|| "Invalid duplicate destination".to_string())?;

    if roots.iter().any(|root| paths_overlap(root, &destination)) {
        return Err("重复项目录不能与资源路径重叠".to_string());
    }

    let mut skipped = 0usize;
    let mut media = Vec::new();
    for root in &roots {
        walk_media(root, &mut media, &mut skipped);
    }

    let mut by_size: HashMap<u64, Vec<_>> = HashMap::new();
    let mut checked = 0usize;
    for path in media {
        let Some(size) = media_size(&path) else {
            skipped += 1;
            continue;
        };
        if size == 0 || size > DEDUPE_MAX_FILE_SIZE {
            skipped += 1;
            continue;
        }
        checked += 1;
        by_size.entry(size).or_default().push(path);
    }

    let mut by_hash: HashMap<(u64, u64), Vec<_>> = HashMap::new();
    for (size, paths) in by_size {
        if paths.len() < 2 {
            continue;
        }
        for path in paths {
            match content_hash(&path) {
                Ok(hash) => {
                    by_hash.entry((size, hash)).or_default().push(path);
                }
                Err(_) => skipped += 1,
            }
        }
    }

    let conn = open_db(&app)?;
    let mut duplicates = 0usize;
    let mut moved = 0usize;
    for (_, mut paths) in by_hash {
        if paths.len() < 2 {
            continue;
        }
        paths.sort();
        let keeper = paths[0].clone();
        for source in paths.into_iter().skip(1) {
            match files_equal(&keeper, &source) {
                Ok(true) => duplicates += 1,
                Ok(false) => continue,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
            let target = unique_destination_path(&destination, &source);
            if move_file(&source, &target).is_err() {
                skipped += 1;
                continue;
            }
            conn.execute(
                "DELETE FROM images WHERE path = ?1",
                params![source.to_string_lossy()],
            )
            .map_err(|err| format!("Failed to remove moved duplicate from database: {err}"))?;
            conn.execute(
                "DELETE FROM image_thumbnails WHERE image_path = ?1",
                params![source.to_string_lossy()],
            )
            .map_err(|err| format!("Failed to remove moved duplicate thumbnail: {err}"))?;
            moved += 1;
        }
    }

    let total = conn
        .query_row("SELECT COUNT(*) FROM images", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("Failed to count images: {err}"))?;

    Ok(DedupeSummary {
        checked,
        duplicates,
        moved,
        skipped,
        total,
        max_file_size: DEDUPE_MAX_FILE_SIZE,
    })
}

pub(crate) fn repair_image_extensions(
    app: tauri::AppHandle,
    paths: Vec<String>,
) -> Result<ExtensionRepairSummary, String> {
    let roots = collect_roots(&paths);
    let mut media = Vec::new();
    let mut skipped = 0usize;
    for root in &roots {
        walk_media(root, &mut media, &mut skipped);
    }

    let conn = open_db(&app)?;
    let updated_at = now_nanos();
    let mut repaired = 0usize;
    for source in media.into_iter().filter(|path| is_supported_image(path)) {
        let Some(format) = image_format(&source) else {
            skipped += 1;
            continue;
        };
        let Some(expected_extension) = image_format_extension(format) else {
            skipped += 1;
            continue;
        };
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("");
        if image_extension_matches(format, extension) {
            continue;
        }
        let target = unique_destination_path(
            source.parent().unwrap_or_else(|| Path::new(".")),
            &source.with_extension(expected_extension),
        );
        if fs::rename(&source, &target).is_err() {
            skipped += 1;
            continue;
        }
        conn.execute(
            "DELETE FROM images WHERE path = ?1",
            params![source.to_string_lossy()],
        )
        .map_err(|err| format!("Failed to update repaired image record: {err}"))?;
        conn.execute(
            "DELETE FROM image_thumbnails WHERE image_path = ?1",
            params![source.to_string_lossy()],
        )
        .map_err(|err| format!("Failed to update repaired thumbnail record: {err}"))?;
        if upsert_image(&conn, &target, updated_at).is_err() {
            skipped += 1;
            continue;
        }
        repaired += 1;
    }

    let total = conn
        .query_row("SELECT COUNT(*) FROM images", [], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|err| format!("Failed to count images: {err}"))?;

    for label in [GALLERY_LABEL, CAROUSEL_LABEL] {
        if let Some(window) = app.get_webview_window(label) {
            let _ = window.eval("window.location.reload()");
        }
    }

    Ok(ExtensionRepairSummary {
        repaired,
        skipped,
        total,
    })
}
