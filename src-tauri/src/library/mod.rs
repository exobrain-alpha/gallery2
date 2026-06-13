//! 素材库系统模块。
//! 负责素材源配置、媒体识别、扫描入库、图库查询、去重修复和缩略图生成。
//! 前端命令编排留在 app::commands，底层路径/数据库能力由 shared/storage 提供。

pub(crate) mod gallery;
pub(crate) mod media;
pub(crate) mod scanner;
pub(crate) mod source_paths;
pub(crate) mod thumbnail;
