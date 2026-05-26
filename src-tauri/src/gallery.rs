use crate::models::{ImageCursor, ImagePage, ImageRecord};
use crate::{configured_thumbnail_dir, open_db, thumbnail_enabled};
use rusqlite::{params, Connection, OptionalExtension};
use std::{fs, path::Path};
use tauri::Manager;

struct ThumbnailRecord {
    path: String,
}

struct RawImageRecord {
    path: String,
    media_type: String,
    width: u32,
    height: u32,
    modified: i64,
    size: i64,
}

pub(crate) fn list_images(
    app: tauri::AppHandle,
    cursor: Option<ImageCursor>,
    limit: i64,
) -> Result<ImagePage, String> {
    let conn = open_db(&app)?;
    let thumbnails_enabled = thumbnail_enabled(&conn)?;
    let thumbnail_dir = thumbnails_enabled
        .then(|| configured_thumbnail_dir(&app, &conn))
        .transpose()?;
    if let Some(dir) = &thumbnail_dir {
        fs::create_dir_all(dir)
            .map_err(|err| format!("Failed to create thumbnail directory: {err}"))?;
        app.asset_protocol_scope()
            .allow_directory(dir, true)
            .map_err(|err| format!("Failed to allow thumbnail directory: {err}"))?;
    }
    let limit = limit.clamp(1, 120);
    let fetch_limit = limit + 1;

    let mut records = if let Some(cursor) = cursor {
        let mut stmt = conn
            .prepare(
                "
                SELECT path, media_type, width, height, modified, size
                FROM images
                WHERE modified < ?1 OR (modified = ?1 AND path COLLATE NOCASE > ?2)
                ORDER BY modified DESC, path COLLATE NOCASE ASC
                LIMIT ?3
                ",
            )
            .map_err(|err| format!("Failed to prepare cursor image query: {err}"))?;
        let records = stmt
            .query_map(
                params![cursor.modified, cursor.path, fetch_limit],
                raw_record_from_row,
            )
            .map_err(|err| format!("Failed to query cursor images: {err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("Failed to collect cursor images: {err}"))?;
        records
    } else {
        let mut stmt = conn
            .prepare(
                "
                SELECT path, media_type, width, height, modified, size
                FROM images
                ORDER BY modified DESC, path COLLATE NOCASE ASC
                LIMIT ?1
                ",
            )
            .map_err(|err| format!("Failed to prepare image query: {err}"))?;
        let records = stmt
            .query_map(params![fetch_limit], raw_record_from_row)
            .map_err(|err| format!("Failed to query images: {err}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| format!("Failed to collect images: {err}"))?;
        records
    };

    let has_more = records.len() > limit as usize;
    if has_more {
        records.truncate(limit as usize);
    }
    let next_cursor = has_more
        .then(|| records.last())
        .flatten()
        .map(|record| ImageCursor {
            modified: record.modified,
            path: record.path.clone(),
        });
    let records = records
        .into_iter()
        .map(|record| image_record_from_raw(&conn, thumbnail_dir.as_deref(), record))
        .collect::<Vec<_>>();

    Ok(ImagePage {
        items: records,
        next_cursor,
    })
}

fn raw_record_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawImageRecord> {
    Ok(RawImageRecord {
        path: row.get(0)?,
        media_type: row.get(1)?,
        width: row.get(2)?,
        height: row.get(3)?,
        modified: row.get(4)?,
        size: row.get(5)?,
    })
}

fn image_record_from_raw(
    conn: &Connection,
    thumbnail_dir: Option<&Path>,
    record: RawImageRecord,
) -> ImageRecord {
    let display_path = if record.media_type == "image" {
        thumbnail_dir
            .and_then(|dir| {
                existing_thumbnail(conn, dir, &record.path, record.modified, record.size).ok()
            })
            .flatten()
            .map(|thumbnail| thumbnail.path)
            .unwrap_or_else(|| record.path.clone())
    } else {
        record.path.clone()
    };

    ImageRecord {
        path: record.path,
        display_path,
        media_type: record.media_type,
        width: record.width,
        height: record.height,
        modified: record.modified,
        size: record.size,
    }
}

fn existing_thumbnail(
    conn: &Connection,
    thumbnail_dir: &Path,
    image_path: &str,
    modified: i64,
    size: i64,
) -> Result<Option<ThumbnailRecord>, String> {
    let record = conn
        .query_row(
            "
        SELECT thumb_path
        FROM image_thumbnails
        WHERE image_path = ?1 AND source_modified = ?2 AND source_size = ?3
        ",
            params![image_path, modified, size],
            |row| Ok(ThumbnailRecord { path: row.get(0)? }),
        )
        .optional()
        .map_err(|err| format!("Failed to query thumbnail record: {err}"))?;

    Ok(record.filter(|record| {
        let path = Path::new(&record.path);
        path.starts_with(thumbnail_dir) && path.is_file()
    }))
}
