//! xAI 图片编辑工作流。
//! 负责 xAI key 设置、参考图读取、编辑请求、生成图片保存和编辑结果归档。
//! 不维护编辑器会话消息；会话持久化由 editor::session 处理。

use crate::{
    library::media::{image_format, image_mime_type, is_supported_image},
    shared::{
        encoding::{base64_decode, base64_encode},
        models::{SavedGeneratedImage, XaiEditArchiveEntry, XaiEditResult, XaiKeyStatus},
        path_utils::{
            canonical_user_path, normalize_path, unique_destination_path, user_path_string,
        },
        time::now_secs,
    },
    storage::{
        asset_scope::{allow_asset_directory, refresh_asset_scope_with_conn},
        config::{
            configured_generated_content_dir, default_generated_content_dir, read_xai_key_config,
            write_xai_key_config,
        },
        db::{open_db, write_config},
        paths::app_data_dir,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tauri::Manager;

fn data_uri_parts(data_uri: &str) -> Result<(&str, &str), String> {
    let Some((header, data)) = data_uri.split_once(',') else {
        return Err("Invalid data URI".to_string());
    };
    let mime_type = header
        .strip_prefix("data:")
        .and_then(|value| value.split_once(';').map(|(mime_type, _)| mime_type))
        .ok_or_else(|| "Invalid data URI header".to_string())?;
    Ok((mime_type, data))
}

fn mime_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => "png",
    }
}

fn first_json_string(value: &serde_json::Value, pointers: &[&str]) -> Option<String> {
    pointers.iter().find_map(|pointer| {
        value
            .pointer(pointer)
            .and_then(|entry| entry.as_str())
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
    })
}

fn format_xai_http_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    let payload = serde_json::from_slice::<serde_json::Value>(body).ok();
    let body_text = String::from_utf8_lossy(body).trim().to_string();
    let code = payload
        .as_ref()
        .and_then(|value| first_json_string(value, &["/code", "/error/code"]));
    let detail = payload
        .as_ref()
        .and_then(|value| first_json_string(value, &["/error", "/message", "/error/message"]));
    let summary = [code.as_deref(), detail.as_deref(), Some(body_text.as_str())]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let normalized = summary.to_ascii_lowercase();

    if normalized.contains("content moderation") || normalized.contains("moderation") {
        return "内容审核未通过，请调整提示词或参考图后重试".to_string();
    }

    if is_xai_billing_error(&normalized) {
        return "xAI 账户额度不足，请充值或调整账单后重试".to_string();
    }

    match status.as_u16() {
        400 => detail
            .or(code)
            .map(|message| format!("xAI 请求失败：{message}"))
            .unwrap_or_else(|| "xAI 请求失败，请检查提示词和参考图".to_string()),
        401 | 403 => "xAI Key 无效或已失效".to_string(),
        408 => "xAI 请求超时，请稍后重试".to_string(),
        429 => "请求过于频繁，请稍后重试".to_string(),
        500..=599 => "xAI 服务暂时不可用，请稍后重试".to_string(),
        _ => detail
            .or(code)
            .map(|message| format!("xAI 请求失败：{message}"))
            .unwrap_or_else(|| format!("xAI 请求失败（HTTP {status}）")),
    }
}

fn is_xai_billing_error(message: &str) -> bool {
    [
        "billing",
        "balance",
        "credit",
        "credits",
        "quota",
        "insufficient",
        "payment",
        "funds",
        "spend",
        "usage limit",
        "额度",
        "余额",
        "费用",
        "充值",
        "账单",
    ]
    .iter()
    .any(|keyword| message.contains(keyword))
}

async fn run_xai_edit_request(
    xai_key: &str,
    request: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://api.x.ai/v1/images/edits")
        .bearer_auth(xai_key)
        .header("Content-Type", "application/json")
        .body(request.to_string())
        .send()
        .await
        .map_err(|err| format!("xAI request failed: {err}"))?;
    let status = response.status();
    let body = response
        .bytes()
        .await
        .map_err(|err| format!("Failed to read xAI response: {err}"))?;
    if !status.is_success() {
        return Err(format_xai_http_error(status, &body));
    }
    serde_json::from_slice(&body).map_err(|err| {
        format!(
            "Failed to parse xAI response: {err}: {}",
            String::from_utf8_lossy(&body)
        )
    })
}

async fn download_url(url: &str) -> Result<Vec<u8>, String> {
    let response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|err| format!("Failed to download generated image: {err}"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|err| format!("Failed to read generated image: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "Failed to download generated image: HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        ));
    }
    Ok(bytes.to_vec())
}

fn output_image_from_response(response: &serde_json::Value) -> Option<String> {
    response
        .pointer("/url")
        .or_else(|| response.pointer("/image/url"))
        .or_else(|| response.pointer("/image_url"))
        .or_else(|| response.pointer("/data/0/url"))
        .or_else(|| response.pointer("/data/0/image_url"))
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .or_else(|| {
            response
                .pointer("/base64")
                .or_else(|| response.pointer("/b64_json"))
                .or_else(|| response.pointer("/image/base64"))
                .or_else(|| response.pointer("/data/0/b64_json"))
                .or_else(|| response.pointer("/data/0/base64"))
                .and_then(|value| value.as_str())
                .map(|base64| {
                    if base64.starts_with("data:") {
                        base64.to_string()
                    } else {
                        format!("data:image/png;base64,{base64}")
                    }
                })
        })
}

fn output_images_from_response(response: &serde_json::Value) -> Vec<String> {
    if let Some(items) = response.pointer("/data").and_then(|value| value.as_array()) {
        let images = items
            .iter()
            .filter_map(output_image_from_response)
            .collect::<Vec<_>>();
        if !images.is_empty() {
            return images;
        }
    }
    output_image_from_response(response).into_iter().collect()
}

fn save_generated_image_bytes(
    app: &tauri::AppHandle,
    bytes: &[u8],
    extension: &str,
    source_path: &str,
) -> Result<SavedGeneratedImage, String> {
    let conn = open_db(app)?;
    let dir = configured_generated_content_dir(app, &conn)?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
    allow_asset_directory(&app.asset_protocol_scope(), &dir)?;
    let source_stem = Path::new(source_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let candidate = dir.join(format!("{source_stem}-xai-{}.{}", now_secs(), extension));
    let target = unique_destination_path(&dir, &candidate);
    fs::write(&target, bytes).map_err(|err| format!("Failed to save generated image: {err}"))?;
    Ok(SavedGeneratedImage {
        path: user_path_string(&target),
    })
}

async fn save_generated_image_source(
    app: &tauri::AppHandle,
    image_source: &str,
    source_path: &str,
) -> Result<SavedGeneratedImage, String> {
    if image_source.starts_with("data:") {
        let (mime_type, base64_data) = data_uri_parts(image_source)?;
        let bytes = base64_decode(base64_data)?;
        return save_generated_image_bytes(app, &bytes, mime_extension(mime_type), source_path);
    }

    let bytes = download_url(image_source).await?;
    let extension = image_source
        .split('?')
        .next()
        .and_then(|path| Path::new(path).extension())
        .and_then(|extension| extension.to_str())
        .filter(|extension| !extension.is_empty())
        .unwrap_or("png");
    save_generated_image_bytes(app, &bytes, extension, source_path)
}

pub(crate) fn save_xai_settings(
    app: tauri::AppHandle,
    xai_key: String,
    generated_content_dir: String,
) -> Result<(), String> {
    let conn = open_db(&app)?;
    let app_dir = canonical_user_path(&app_data_dir(&app)?)?;
    let default_dir = default_generated_content_dir(&app)?;
    let dir = if generated_content_dir.trim().is_empty() {
        default_dir
    } else {
        PathBuf::from(generated_content_dir.trim())
    };
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
    let mut dir = canonical_user_path(&dir)
        .map_err(|err| format!("Failed to canonicalize generated content directory: {err}"))?;
    if dir == app_dir {
        dir = default_generated_content_dir(&app)?;
        fs::create_dir_all(&dir)
            .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
        dir = canonical_user_path(&dir)
            .map_err(|err| format!("Failed to canonicalize generated content directory: {err}"))?;
    }
    write_xai_key_config(&app, &conn, &xai_key)?;
    write_config(&conn, "generated_content_dir", &user_path_string(&dir))?;
    refresh_asset_scope_with_conn(&app, &conn)?;
    Ok(())
}

pub(crate) fn get_xai_key_status(app: tauri::AppHandle) -> Result<XaiKeyStatus, String> {
    let conn = open_db(&app)?;
    let xai_key = read_xai_key_config(&app, &conn)?;
    Ok(XaiKeyStatus {
        configured: !xai_key.trim().is_empty(),
    })
}

pub(crate) fn read_image_data_uri(path: String) -> Result<String, String> {
    let path = normalize_path(&path).ok_or_else(|| "Invalid image path".to_string())?;
    if !path.is_file() || !is_supported_image(&path) {
        return Err("Unsupported image".to_string());
    }
    let format = image_format(&path).ok_or_else(|| "Unsupported image".to_string())?;
    let mime_type = image_mime_type(format).ok_or_else(|| "Unsupported image".to_string())?;
    let bytes = fs::read(&path).map_err(|err| format!("Failed to read image: {err}"))?;
    Ok(format!("data:{mime_type};base64,{}", base64_encode(&bytes)))
}

pub(crate) fn save_generated_image(
    app: tauri::AppHandle,
    data_uri: String,
    source_path: String,
) -> Result<SavedGeneratedImage, String> {
    let conn = open_db(&app)?;
    let dir = configured_generated_content_dir(&app, &conn)?;
    fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create generated content directory: {err}"))?;
    allow_asset_directory(&app.asset_protocol_scope(), &dir)?;
    let source_stem = Path::new(&source_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let (mime_type, base64_data) = data_uri_parts(&data_uri)?;
    let bytes = base64_decode(base64_data)?;
    let extension = mime_extension(mime_type);
    let candidate = dir.join(format!("{source_stem}-xai-{}.{}", now_secs(), extension));
    let target = unique_destination_path(&dir, &candidate);
    fs::write(&target, bytes).map_err(|err| format!("Failed to save generated image: {err}"))?;
    Ok(SavedGeneratedImage {
        path: user_path_string(&target),
    })
}

pub(crate) fn archive_xai_edit(
    app: tauri::AppHandle,
    entry: XaiEditArchiveEntry,
) -> Result<(), String> {
    let dir = app_data_dir(&app)?.join("xai-edit-archives");
    fs::create_dir_all(&dir).map_err(|err| format!("Failed to create archive directory: {err}"))?;
    let source_stem = Path::new(&entry.source_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    let archive_path = unique_destination_path(
        &dir,
        &dir.join(format!("{source_stem}-{}.json", entry.created_at)),
    );
    let json = serde_json::to_vec_pretty(&entry)
        .map_err(|err| format!("Failed to serialize archive: {err}"))?;
    fs::write(archive_path, json).map_err(|err| format!("Failed to write archive: {err}"))?;
    Ok(())
}

pub(crate) async fn edit_image_with_xai(
    app: tauri::AppHandle,
    source_paths: Vec<String>,
    source_data_uris: Vec<String>,
    prompt: String,
    aspect_ratio: Option<String>,
    resolution: Option<String>,
    image_count: Option<u8>,
) -> Result<XaiEditResult, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("提示词不能为空".to_string());
    }
    if source_paths.is_empty() || source_data_uris.is_empty() {
        return Err("参考图不能为空".to_string());
    }
    if source_paths.len() != source_data_uris.len() {
        return Err("参考图数据不完整".to_string());
    }

    let conn = open_db(&app)?;
    let xai_key = read_xai_key_config(&app, &conn)?;
    if xai_key.trim().is_empty() {
        return Err("xAI Key 未设置".to_string());
    }

    let source_path = source_paths
        .first()
        .cloned()
        .ok_or_else(|| "参考图不能为空".to_string())?;
    let source_label = Path::new(&source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    let image_inputs = source_data_uris
        .iter()
        .take(3)
        .map(|uri| {
            serde_json::json!({
                "type": "image_url",
                "url": uri,
            })
        })
        .collect::<Vec<_>>();
    let mut request = serde_json::json!({
        "model": "grok-imagine-image-quality",
        "prompt": prompt,
    });
    let image_count = image_count.unwrap_or(1).clamp(1, 4);
    request["n"] = serde_json::Value::Number(serde_json::Number::from(image_count));
    let aspect_ratio = normalize_optional_text(aspect_ratio).filter(|value| value != "auto");
    if let Some(ratio) = &aspect_ratio {
        validate_xai_image_aspect_ratio(ratio)?;
        request["aspect_ratio"] = serde_json::Value::String(ratio.clone());
    }
    let resolution = normalize_optional_text(resolution);
    if let Some(size) = &resolution {
        validate_xai_image_resolution(size)?;
        request["resolution"] = serde_json::Value::String(size.clone());
    }
    if image_inputs.len() == 1 {
        request["image"] = image_inputs[0].clone();
    } else {
        request["images"] = serde_json::Value::Array(image_inputs);
    }
    let response = run_xai_edit_request(xai_key.trim(), &request).await?;
    let image_sources = output_images_from_response(&response);
    if image_sources.is_empty() {
        return Err("未返回图片".to_string());
    }
    let mut output_paths = Vec::with_capacity(image_sources.len());
    for image_source in image_sources {
        let saved = save_generated_image_source(&app, &image_source, &source_path).await?;
        output_paths.push(saved.path);
    }
    let output_path = output_paths
        .first()
        .cloned()
        .ok_or_else(|| "未返回图片".to_string())?;
    let created_at = now_secs();
    archive_xai_edit(
        app,
        XaiEditArchiveEntry {
            source_path,
            source_label,
            prompt: prompt.to_string(),
            aspect_ratio,
            resolution,
            image_count,
            output_path: Some(output_path.clone()),
            output_paths: output_paths.clone(),
            response: response.clone(),
            created_at,
        },
    )?;
    Ok(XaiEditResult {
        path: output_path,
        paths: output_paths,
        response,
    })
}

fn normalize_optional_text(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn validate_xai_image_aspect_ratio(aspect_ratio: &str) -> Result<(), String> {
    if matches!(
        aspect_ratio,
        "1:1"
            | "16:9"
            | "9:16"
            | "4:3"
            | "3:4"
            | "3:2"
            | "2:3"
            | "2:1"
            | "1:2"
            | "20:9"
            | "9:20"
            | "auto"
    ) {
        Ok(())
    } else {
        Err("不支持的图片比例".to_string())
    }
}

fn validate_xai_image_resolution(resolution: &str) -> Result<(), String> {
    if matches!(resolution, "1k" | "2k") {
        Ok(())
    } else {
        Err("不支持的图片分辨率".to_string())
    }
}
