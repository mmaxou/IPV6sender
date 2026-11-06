//! Bark 推送通知（对应 Python 版 format_ipv6_message / build_bark_payload / send_notification）。

use crate::config::Config;
use crate::ipv6::IpItem;
use chrono::Local;
use serde_json::{json, Value};
use std::time::Duration;

/// 将 IPv6 地址列表格式化为通知消息纯文本。
pub fn format_ipv6_message(addrs: &[IpItem]) -> String {
    let now_str = Local::now().format("%Y-%m-%d %H:%M").to_string();
    if addrs.is_empty() {
        return format!("{now_str} 未检测到IPv6地址");
    }
    let mut lines = vec![now_str];
    for item in addrs {
        lines.push(item.address.clone());
    }
    lines.join("\n")
}

/// 构建 Bark API 的 JSON payload。
pub fn build_bark_payload(title: &str, addrs: &[IpItem], notif: &crate::config::NotificationConfig) -> Value {
    let body = format_ipv6_message(addrs);
    let mut payload = json!({
        "title": title,
        "body": body,
    });

    if !notif.group.is_empty() {
        payload["group"] = Value::String(notif.group.clone());
    }
    if !notif.sound.is_empty() {
        payload["sound"] = Value::String(notif.sound.clone());
    }
    if !notif.icon.is_empty() {
        payload["icon"] = Value::String(notif.icon.clone());
    }
    if !notif.level.is_empty() {
        payload["level"] = Value::String(notif.level.clone());
    }
    payload
}

/// 通过 Bark API 发送推送通知，返回是否成功。
pub fn send_notification(
    cfg: &Config,
    addrs: &[IpItem],
    title_override: Option<&str>,
) -> bool {
    let server = cfg.bark.server.trim_end_matches('/');
    let url = format!("{}/{}", server, cfg.bark.key);
    let title = title_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.notification.title.clone());

    let payload = build_bark_payload(&title, addrs, &cfg.notification);
    let body = serde_json::to_string(&payload).unwrap_or_default();

    let timeout = Duration::from_secs(std::cmp::max(cfg.bark.timeout, 1));
    log::info!("正在发送 Bark 推送到 {} ...", url);

    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let resp = agent
        .post(&url)
        .header("Content-Type", "application/json; charset=utf-8")
        .header("User-Agent", "IPsender/1.0")
        .send_string(&body);

    match resp {
        Ok(r) => {
            let status = r.status();
            log::debug!("Bark 响应 [{}]", status);
            if (200..300).contains(&status) {
                log::info!("Bark 推送发送成功 (HTTP {}).", status);
                true
            } else {
                log::error!("Bark 返回非成功状态码: {}", status);
                false
            }
        }
        Err(ureq::Error::Status(code, _)) => {
            log::error!("Bark 请求失败 (HTTP {})", code);
            false
        }
        Err(e) => {
            log::error!("无法连接到 Bark 服务器: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NotificationConfig;

    #[test]
    fn message_format_empty() {
        assert!(format_ipv6_message(&[]).contains("未检测到IPv6地址"));
    }

    #[test]
    fn message_format_has_addresses() {
        let items = vec![IpItem {
            interface: "N/A".to_string(),
            address: "240e::1".to_string(),
            scope: "global".to_string(),
        }];
        let msg = format_ipv6_message(&items);
        assert!(msg.contains("240e::1"));
    }

    #[test]
    fn payload_fields() {
        let ni = NotificationConfig::default();
        let v = build_bark_payload("标题", &[], &ni);
        assert_eq!(v["title"], "标题");
        assert_eq!(v["body"], format_ipv6_message(&[]));
        // 空的可选字段不应出现在 payload 中
        assert!(v.get("group").is_none());
        assert!(v.get("sound").is_none());
    }
}
