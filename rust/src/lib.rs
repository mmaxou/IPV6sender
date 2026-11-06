//! IPsender —— 自动获取本机 IPv6 地址并通过 Bark 推送通知（Rust 重写版）。
//!
//! 对应原 Python 版本 `ipv6_sender.py` 的全部核心功能：
//! 1. 获取本机 IPv6 地址（可过滤链路本地、回环、按后缀过滤）。
//! 2. 通过 Bark API 推送通知到 iPhone / iPad。
//! 3. 守护进程模式支持：每日定时发送 + IP 变化检测。
//!
//! 仅依赖少数便携 crate，核心逻辑与 Python 版保持行为等价。

pub mod bark;
pub mod config;
pub mod daemon;
pub mod ipv6;
pub mod scheduler;
pub mod state;

use std::path::PathBuf;

/// 运行时基目录：优先取可执行文件所在目录（打包/部署时配置与二进制同目录）。
pub fn base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
}

/// 默认配置文件路径。
pub fn default_config_path() -> PathBuf {
    base_dir().join("config.json")
}

/// 默认状态文件路径。
pub fn default_state_path() -> PathBuf {
    base_dir().join("state.json")
}

/// 执行一次「获取 IPv6 → Bark 推送」的完整流程（等价于 Python 的 run_task）。
pub fn run_task(cfg: &config::Config) {
    log::info!("{}", "=".repeat(50));
    log::info!("开始执行任务...");

    let addrs = ipv6::get_filtered_addrs(cfg);

    if addrs.is_empty() {
        log::warn!("未检测到任何 IPv6 地址。");
    } else {
        log::info!("检测到 {} 个 IPv6 地址:", addrs.len());
        for item in &addrs {
            log::info!("  {} ({})", item.address, item.scope);
        }
    }

    bark::send_notification(cfg, &addrs, None);
    log::info!("任务执行完毕。");
}
