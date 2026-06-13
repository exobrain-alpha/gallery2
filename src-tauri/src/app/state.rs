//! 后端共享运行时状态。
//! 定义缩略图进度、Windows 全屏恢复标记和走马灯全屏时的 keep-awake 状态。
//! 只管理内存状态及平台保持唤醒调用，不处理窗口创建。

use crate::shared::models::ThumbnailProgress;
#[cfg(target_os = "macos")]
use std::process::{Child, Command, Stdio};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

pub(crate) type ThumbnailProgressState = Arc<Mutex<ThumbnailProgress>>;
pub(crate) type WindowsFullscreenRestoreState = Arc<Mutex<HashSet<String>>>;
pub(crate) type KeepAwakeState = Arc<Mutex<KeepAwake>>;

#[derive(Default)]
pub(crate) struct KeepAwake {
    #[cfg(target_os = "macos")]
    caffeinate: Option<Child>,
    #[cfg(target_os = "windows")]
    active: bool,
}

impl KeepAwake {
    pub(crate) fn set_active(&mut self, active: bool) -> Result<(), String> {
        self.set_platform_active(active)
    }

    #[cfg(target_os = "macos")]
    fn set_platform_active(&mut self, active: bool) -> Result<(), String> {
        if active {
            let needs_spawn = match self.caffeinate.as_mut() {
                Some(child) => child
                    .try_wait()
                    .map_err(|err| format!("Failed to check caffeinate state: {err}"))?
                    .is_some(),
                None => true,
            };
            if !needs_spawn {
                return Ok(());
            }

            let pid = std::process::id().to_string();
            let child = Command::new("/usr/bin/caffeinate")
                .args(["-d", "-i", "-u", "-t", "86400", "-w", &pid])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|err| format!("Failed to start caffeinate: {err}"))?;
            self.caffeinate = Some(child);
            return Ok(());
        }

        if let Some(mut child) = self.caffeinate.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    fn set_platform_active(&mut self, active: bool) -> Result<(), String> {
        if self.active == active {
            return Ok(());
        }

        const ES_CONTINUOUS: u32 = 0x8000_0000;
        const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
        const ES_DISPLAY_REQUIRED: u32 = 0x0000_0002;

        #[link(name = "kernel32")]
        extern "system" {
            fn SetThreadExecutionState(es_flags: u32) -> u32;
        }

        let flags = if active {
            ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED
        } else {
            ES_CONTINUOUS
        };
        let result = unsafe { SetThreadExecutionState(flags) };
        if result == 0 {
            return Err("Failed to update execution state".to_string());
        }

        self.active = active;
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn set_platform_active(&mut self, _active: bool) -> Result<(), String> {
        Ok(())
    }
}
