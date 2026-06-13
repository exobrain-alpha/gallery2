//! Gallery 桌面应用二进制入口。
//! 只负责应用进程启动并调用 gallery2_lib::run()。
//! Tauri 初始化和业务逻辑都在库模块中维护。

// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    gallery2_lib::run()
}
