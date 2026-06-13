//! 图片编辑系统模块。
//! 管理编辑器会话持久化和 xAI 图片编辑工作流。
//! 会话存储与外部 AI 请求分开维护，避免网络流程污染本地状态管理。

pub(crate) mod session;
pub(crate) mod xai;
