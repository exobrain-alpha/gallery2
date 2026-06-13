//! 图片缩略图生成的底层实现。
//! 负责根据源文件信息生成缓存键，并把源图片解码后写成受控尺寸的 JPEG 缩略图。
//! 不负责选择生成哪些缩略图或记录数据库状态。

use crate::shared::encoding::hex_encode;
use image::{codecs::jpeg::JpegEncoder, ImageReader};
use sha2::{Digest, Sha256};
use std::{fs, path::Path};

#[allow(dead_code)]
pub(crate) const THUMBNAIL_MAX_EDGE: u32 = 768;
#[allow(dead_code)]
pub(crate) const THUMBNAIL_QUALITY: u8 = 82;

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
