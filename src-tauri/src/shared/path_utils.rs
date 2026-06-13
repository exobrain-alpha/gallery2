//! 本地路径和文件移动工具。
//! 负责用户可读路径规范化、Windows verbatim 路径清理、唯一目标路径生成和跨卷移动。
//! 不读取应用配置，也不决定哪些目录属于素材库。

use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn user_path_string(path: &Path) -> String {
    clean_windows_verbatim_path(&path.to_string_lossy())
}

pub(crate) fn user_path_buf(path: PathBuf) -> PathBuf {
    PathBuf::from(user_path_string(&path))
}

pub(crate) fn canonical_user_path(path: &Path) -> Result<PathBuf, String> {
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
