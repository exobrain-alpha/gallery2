//! 应用更新服务。
//! 封装 Tauri updater 插件的检查、下载、安装和重启流程。

use crate::shared::models::AppUpdateInfo;
use tauri_plugin_updater::{Error as UpdaterError, UpdaterExt};

#[derive(Debug)]
pub(crate) struct AppUpdateRuntimeState {
    init_error: Option<String>,
}

impl AppUpdateRuntimeState {
    fn ensure_ready(&self) -> Result<(), String> {
        match &self.init_error {
            Some(error) => Err(format!("更新服务初始化失败: {error}")),
            None => Ok(()),
        }
    }
}

pub(crate) fn initialize(app: &tauri::AppHandle) -> AppUpdateRuntimeState {
    match app.plugin(tauri_plugin_updater::Builder::new().build()) {
        Ok(()) => AppUpdateRuntimeState { init_error: None },
        Err(error) => {
            let error = error.to_string();
            eprintln!("Failed to initialize updater plugin: {error}");
            AppUpdateRuntimeState {
                init_error: Some(error),
            }
        }
    }
}

pub(crate) async fn check(
    app: tauri::AppHandle,
    state: &AppUpdateRuntimeState,
) -> Result<AppUpdateInfo, String> {
    state.ensure_ready()?;

    let current_version = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(format_updater_error)?
        .check()
        .await
        .map_err(format_updater_error)?;

    Ok(match update {
        Some(update) => AppUpdateInfo {
            available: true,
            current_version: update.current_version.clone(),
            version: Some(update.version.clone()),
            date: update.date.map(|date| date.to_string()),
            body: update.body.clone(),
        },
        None => AppUpdateInfo {
            available: false,
            current_version,
            version: None,
            date: None,
            body: None,
        },
    })
}

pub(crate) async fn install(
    app: tauri::AppHandle,
    state: &AppUpdateRuntimeState,
) -> Result<(), String> {
    state.ensure_ready()?;

    let update = app
        .updater()
        .map_err(format_updater_error)?
        .check()
        .await
        .map_err(format_updater_error)?
        .ok_or_else(|| "当前没有可安装更新".to_string())?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(format_updater_error)?;
    app.restart();
}

fn format_updater_error(error: UpdaterError) -> String {
    match error {
        UpdaterError::EmptyEndpoints => "未配置更新服务".to_string(),
        UpdaterError::InsecureTransportProtocol => "更新服务必须使用 HTTPS endpoint".to_string(),
        UpdaterError::ReleaseNotFound => "未找到有效的更新信息".to_string(),
        UpdaterError::UnsupportedArch | UpdaterError::UnsupportedOs => {
            "当前平台不支持应用更新".to_string()
        }
        error => error.to_string(),
    }
}
