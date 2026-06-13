//! Gallery Tauri 后端库的模块装配入口。
//! 只声明顶层系统模块并把 public run() 转发给 app 模块。
//! 新增能力应先归入一个系统域，再在域内按功能拆分。

mod app;
mod editor;
mod library;
mod shared;
mod storage;
mod window;

pub fn run() {
    app::run();
}
