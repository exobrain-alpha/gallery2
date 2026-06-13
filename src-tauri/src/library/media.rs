//! 媒体文件的底层处理能力。
//! 负责图片/视频类型识别、目录遍历、尺寸读取、内容比较和 images 表写入。
//! 不负责前端命令编排或用户路径配置。

use crate::shared::path_utils::user_path_string;
use image::{ImageFormat, ImageReader};
use rusqlite::{params, Connection};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

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

pub(crate) fn image_mime_type(format: ImageFormat) -> Option<&'static str> {
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
