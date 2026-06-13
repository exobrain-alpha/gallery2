//! 后端统一使用的时间戳工具。
//! 提供秒级和纳秒级时间戳，供数据库更新时间、文件命名和会话 id 使用。
//! 不处理时区、格式化展示或用户可见日期。

use std::time::UNIX_EPOCH;

pub(crate) fn now_secs() -> i64 {
    UNIX_EPOCH
        .elapsed()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn now_nanos() -> i64 {
    UNIX_EPOCH
        .elapsed()
        .map(|duration| i64::try_from(duration.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}
