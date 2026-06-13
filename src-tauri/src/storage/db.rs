//! SQLite 连接和 schema 管理。
//! 负责打开数据库、创建和迁移表结构，以及 app_config 通用读写。
//! 不承载具体业务查询；图库、扫描和配置模块各自维护自己的 SQL 行为。

use crate::storage::paths::db_path;
use rusqlite::{params, Connection};

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
