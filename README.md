# Gallery

Gallery 是一个基于 Tauri 2、React、TypeScript 和 SQLite 的本地媒体图库应用。官网：[https://aihouzi.cn/gallery](https://aihouzi.cn/gallery)。

它支持本地图片和视频目录管理、媒体扫描、瀑布流图库浏览、全屏预览、走马灯展示、桌面背景模式、缩略图生成、重复资源整理、图片扩展名修复、xAI 图片编辑、应用设置和更新检查。应用通过 Rust 后端统一管理窗口、文件访问、图库扫描、缩略图、持久化配置和系统集成能力，前端负责图库、走马灯、设置和图片编辑等交互界面。

## 技术栈

- 桌面框架：Tauri 2
- 前端：React + TypeScript + Vite
- 后端：Rust
- 数据库：SQLite3 + rusqlite

## 开发环境

开始开发前，请先按 [Tauri 2 Prerequisites](https://v2.tauri.app/start/prerequisites/) 安装本机依赖：

- Node.js 24 LTS 或更高版本，以及 npm
- Rust stable 工具链（建议通过 rustup 安装）
- macOS：Xcode 或 Xcode Command Line Tools
- Windows：Microsoft C++ Build Tools 和 WebView2 Runtime
- Linux：对应发行版的 WebKitGTK、构建工具和系统库

安装项目依赖：

```sh
npm install
```

## 开发模式

启动完整 [Tauri 桌面开发模式](https://v2.tauri.app/develop/#developing-your-desktop-application)：

```sh
npm run dev:app
```

该命令会通过 Tauri CLI 启动 Vite 开发服务器，并打开桌面应用窗口。首次运行时 Rust 依赖编译可能需要较长时间，后续启动会复用缓存。

如果只需要调试纯前端页面，可以运行：

```sh
npm run dev
```

纯前端模式不包含 Tauri 运行时能力，涉及窗口、文件系统、图库扫描、更新等 `invoke` 调用的功能需要在 `npm run dev:app` 中验证。

## 许可

本项目使用 MIT License，详见 [LICENSE](./LICENSE)。
