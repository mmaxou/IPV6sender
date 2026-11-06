//! 状态文件持久化：记录上次已知 IPv6 地址（对应 Python 版 load_last_ip / save_last_ip）。

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
struct State {
    last_ip: String,
    updated_at: String,
}

/// 从状态文件加载上次已知的 IPv6 地址；文件不存在或损坏时返回空串。
pub fn load_last_ip<P: AsRef<Path>>(state_path: P) -> String {
    let path = state_path.as_ref();
    if !path.exists() {
        return String::new();
    }
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<State>(&text) {
            Ok(state) => state.last_ip,
            Err(_) => {
                log::warn!("读取状态文件失败: JSON 解析错误");
                String::new()
            }
        },
        Err(e) => {
            log::warn!("读取状态文件失败: {e}");
            String::new()
        }
    }
}

/// 将当前 IPv6 地址保存到状态文件。
pub fn save_last_ip<P: AsRef<Path>>(state_path: P, ip: &str) {
    let state = State {
        last_ip: ip.to_string(),
        updated_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };
    match serde_json::to_string_pretty(&state) {
        Ok(text) => {
            if let Err(e) = fs::write(state_path.as_ref(), text) {
                log::warn!("写入状态文件失败: {e}");
            }
        }
        Err(e) => log::warn!("序列化状态失败: {e}"),
    }
}
