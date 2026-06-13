//! 图片编辑会话的本地存储模块。
//! 负责生成、校验 session id，并把长对话按 segment 文件写入应用数据目录。
//! 不处理 xAI 请求或图片保存，只维护编辑器会话状态。

use crate::{
    shared::{
        models::{
            EditorSessionMessage, EditorSessionMeta, EditorSessionSegment, EditorSessionState,
        },
        time::now_secs,
    },
    storage::paths::app_data_dir,
};
use std::time::UNIX_EPOCH;
use std::{fs, path::PathBuf};

const EDITOR_SESSION_SEGMENT_TURNS: usize = 100;

fn editor_sessions_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app_data_dir(app)?.join("editor-sessions");
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create editor sessions directory: {err}"))?;
    Ok(dir)
}

fn editor_session_meta_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(editor_sessions_dir(app)?.join("meta.json"))
}

fn new_editor_session_id() -> String {
    let nanos = UNIX_EPOCH
        .elapsed()
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("session-{nanos}")
}

fn validate_editor_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || !session_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
    {
        return Err("Invalid editor session id".to_string());
    }
    Ok(())
}

fn write_editor_session_meta(app: &tauri::AppHandle, session_id: &str) -> Result<(), String> {
    validate_editor_session_id(session_id)?;
    let meta = EditorSessionMeta {
        current_session_id: session_id.to_string(),
        updated_at: now_secs(),
    };
    let json = serde_json::to_vec_pretty(&meta)
        .map_err(|err| format!("Failed to serialize editor session metadata: {err}"))?;
    fs::write(editor_session_meta_path(app)?, json)
        .map_err(|err| format!("Failed to write editor session metadata: {err}"))?;
    Ok(())
}

fn current_editor_session_id(app: &tauri::AppHandle) -> Result<String, String> {
    let meta_path = editor_session_meta_path(app)?;
    if meta_path.exists() {
        let bytes = fs::read(&meta_path)
            .map_err(|err| format!("Failed to read editor session metadata: {err}"))?;
        if let Ok(meta) = serde_json::from_slice::<EditorSessionMeta>(&bytes) {
            if validate_editor_session_id(&meta.current_session_id).is_ok() {
                return Ok(meta.current_session_id);
            }
        }
    }

    let session_id = new_editor_session_id();
    write_editor_session_meta(app, &session_id)?;
    Ok(session_id)
}

pub(crate) fn load_editor_session(app: tauri::AppHandle) -> Result<EditorSessionState, String> {
    let session_id = current_editor_session_id(&app)?;
    let session_dir = editor_sessions_dir(&app)?.join(&session_id);
    if !session_dir.exists() {
        return Ok(EditorSessionState {
            session_id,
            messages: Vec::new(),
        });
    }

    let mut segment_paths = fs::read_dir(&session_dir)
        .map_err(|err| format!("Failed to read editor session directory: {err}"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.starts_with("segment-") && name.ends_with(".json"))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    segment_paths.sort();

    let mut messages = Vec::new();
    for path in segment_paths {
        let bytes = fs::read(&path)
            .map_err(|err| format!("Failed to read editor session segment: {err}"))?;
        let segment = serde_json::from_slice::<EditorSessionSegment>(&bytes)
            .map_err(|err| format!("Failed to parse editor session segment: {err}"))?;
        if segment.session_id == session_id {
            messages.extend(segment.messages);
        }
    }

    Ok(EditorSessionState {
        session_id,
        messages,
    })
}

pub(crate) fn save_editor_session(
    app: tauri::AppHandle,
    session_id: String,
    messages: Vec<EditorSessionMessage>,
) -> Result<(), String> {
    validate_editor_session_id(&session_id)?;
    write_editor_session_meta(&app, &session_id)?;
    let session_dir = editor_sessions_dir(&app)?.join(&session_id);
    fs::create_dir_all(&session_dir)
        .map_err(|err| format!("Failed to create editor session directory: {err}"))?;

    for entry in fs::read_dir(&session_dir)
        .map_err(|err| format!("Failed to inspect editor session directory: {err}"))?
        .flatten()
    {
        let path = entry.path();
        let is_segment = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.starts_with("segment-") && name.ends_with(".json"))
            .unwrap_or(false);
        if is_segment {
            fs::remove_file(path)
                .map_err(|err| format!("Failed to remove old editor session segment: {err}"))?;
        }
    }

    let now = now_secs();
    let mut segments: Vec<Vec<EditorSessionMessage>> = Vec::new();
    let mut current_segment: Vec<EditorSessionMessage> = Vec::new();
    let mut turn_count = 0usize;

    for message in messages {
        if message.role == "user" && turn_count >= EDITOR_SESSION_SEGMENT_TURNS {
            segments.push(current_segment);
            current_segment = Vec::new();
            turn_count = 0;
        }
        if message.role == "user" {
            turn_count += 1;
        }
        current_segment.push(message);
    }

    if !current_segment.is_empty() {
        segments.push(current_segment);
    }

    for (segment_index, segment_messages) in segments.into_iter().enumerate() {
        let segment = EditorSessionSegment {
            session_id: session_id.clone(),
            segment_index,
            messages: segment_messages,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_vec_pretty(&segment)
            .map_err(|err| format!("Failed to serialize editor session segment: {err}"))?;
        let segment_path = session_dir.join(format!("segment-{segment_index:05}.json"));
        fs::write(segment_path, json)
            .map_err(|err| format!("Failed to write editor session segment: {err}"))?;
    }

    Ok(())
}
