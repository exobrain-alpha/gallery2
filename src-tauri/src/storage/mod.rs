//! 配置与持久化系统模块。
//! 管理应用数据路径、SQLite schema、app_config 配置和 asset 协议访问范围。
//! 业务查询和扫描流程不放在这里，只提供稳定的存储与配置能力。

pub(crate) mod asset_scope;
pub(crate) mod config;
pub(crate) mod db;
pub(crate) mod paths;
