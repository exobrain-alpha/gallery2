//! 跨系统共享模块。
//! 存放前后端协议模型、时间戳、路径工具和轻量编码工具。
//! 这里不放业务流程，只放多个系统域都会依赖的稳定基础能力。

pub(crate) mod encoding;
pub(crate) mod models;
pub(crate) mod path_utils;
pub(crate) mod time;
