//! 前后端交互使用的数据结构定义。
//! 保存 Tauri command 入参/出参、设置状态、图库记录、扫描结果和编辑会话模型。
//! 这里不放行为逻辑，字段命名需保持与前端 camelCase 协议一致。

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageRecord {
    pub(crate) path: String,
    pub(crate) display_path: String,
    pub(crate) media_type: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) modified: i64,
    pub(crate) size: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageCursor {
    pub(crate) modified: i64,
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImagePage {
    pub(crate) items: Vec<ImageRecord>,
    pub(crate) next_cursor: Option<ImageCursor>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsState {
    pub(crate) platform: String,
    pub(crate) paths: Vec<String>,
    pub(crate) image_count: i64,
    pub(crate) db_path: String,
    pub(crate) generated_content_dir: String,
    pub(crate) thumbnail_enabled: bool,
    pub(crate) thumbnail_dir: String,
    pub(crate) xai_key: String,
    pub(crate) gallery_mode: String,
    pub(crate) gallery_has_gap: bool,
    pub(crate) gallery_theme: String,
    pub(crate) min_column_width: u32,
    pub(crate) windows_close_behavior: String,
    pub(crate) windows_startup_enabled: bool,
    pub(crate) windows_startup_desktop_background: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WindowsStartupSettings {
    pub(crate) startup_enabled: bool,
    pub(crate) startup_desktop_background: bool,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XaiEditArchiveEntry {
    pub(crate) source_path: String,
    pub(crate) source_label: String,
    pub(crate) prompt: String,
    pub(crate) aspect_ratio: Option<String>,
    pub(crate) resolution: Option<String>,
    pub(crate) image_count: u8,
    pub(crate) output_path: Option<String>,
    pub(crate) output_paths: Vec<String>,
    pub(crate) response: serde_json::Value,
    pub(crate) created_at: i64,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorSessionAttachment {
    pub(crate) path: String,
    pub(crate) label: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorSessionMessage {
    pub(crate) id: String,
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) attachments: Vec<EditorSessionAttachment>,
    pub(crate) tone: Option<String>,
    pub(crate) aspect_ratio: Option<String>,
    pub(crate) resolution: Option<String>,
    pub(crate) image_count: Option<u8>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorSessionMeta {
    pub(crate) current_session_id: String,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorSessionState {
    pub(crate) session_id: String,
    pub(crate) messages: Vec<EditorSessionMessage>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorSessionSegment {
    pub(crate) session_id: String,
    pub(crate) segment_index: usize,
    pub(crate) messages: Vec<EditorSessionMessage>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedGeneratedImage {
    pub(crate) path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PickedImage {
    pub(crate) path: String,
    pub(crate) data_uri: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XaiEditResult {
    pub(crate) path: String,
    pub(crate) paths: Vec<String>,
    pub(crate) response: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct XaiKeyStatus {
    pub(crate) configured: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GalleryPreferences {
    pub(crate) has_gap: bool,
    pub(crate) theme: String,
    pub(crate) min_column_width: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourcePathsUpdate {
    pub(crate) paths: Vec<String>,
    pub(crate) changed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScanSummary {
    pub(crate) indexed: usize,
    pub(crate) skipped: usize,
    pub(crate) removed: usize,
    pub(crate) total: i64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ThumbnailProgress {
    pub(crate) running: bool,
    pub(crate) stage: String,
    pub(crate) processed: usize,
    pub(crate) total: usize,
    pub(crate) generated: usize,
    pub(crate) skipped: usize,
    pub(crate) message: String,
    pub(crate) error: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DedupeSummary {
    pub(crate) checked: usize,
    pub(crate) duplicates: usize,
    pub(crate) moved: usize,
    pub(crate) skipped: usize,
    pub(crate) total: i64,
    pub(crate) max_file_size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExtensionRepairSummary {
    pub(crate) repaired: usize,
    pub(crate) skipped: usize,
    pub(crate) total: i64,
}
