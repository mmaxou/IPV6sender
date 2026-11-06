//! IPv6 地址获取与过滤（对应 Python 版 ipv6_sender.py 的地址部分）。
//!
//! 获取策略（多方法回退）：
//! 1. Windows: 解析 `ipconfig /all` 输出（最可靠，能覆盖所有网卡）。
//! 2. socket 回退：通过主机名解析 IPv6 地址。

use crate::config::Config;
use std::net::{Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::process::Command;

/// 一条 IPv6 地址记录，结构与原 Python 版保持一致。
#[derive(Debug, Clone)]
pub struct IpItem {
    pub interface: String,
    pub address: String,
    pub scope: String,
}

/// 判断是否为链路本地地址 (fe80::/10)。
fn is_link_local(addr: &str) -> bool {
    addr.to_lowercase().starts_with("fe80:")
}

/// 判断是否为回环地址 (::1)。
fn is_loopback(addr: &str) -> bool {
    addr.eq_ignore_ascii_case("::1")
}

/// 判断是否为全局单播地址（2000::/3，即以 2 或 3 开头）。
fn is_global(addr: &str) -> bool {
    let first = addr.split(':').next().unwrap_or("").to_lowercase();
    first.starts_with('2') || first.starts_with('3')
}

/// 去掉 %zone 标识，返回地址本体。
fn strip_zone(addr: &str) -> &str {
    addr.split('%').next().unwrap_or(addr)
}

/// 简单校验并规范化 IPv6 地址字符串；不合法时返回 None。
fn parse_ipv6(addr_str: &str) -> Option<String> {
    let s = addr_str.trim().split('%').next().unwrap_or(addr_str).trim();
    if s.is_empty() || s.contains('.') || !s.contains(':') {
        return None;
    }
    match s.parse::<Ipv6Addr>() {
        Ok(_) => Some(s.to_string()),
        Err(_) => None,
    }
}

fn scope_for(addr: &str) -> String {
    if is_global(addr) {
        "global".to_string()
    } else if is_link_local(addr) {
        "link_local".to_string()
    } else {
        "other".to_string()
    }
}

/// 通过 Windows ipconfig 命令获取 IPv6 地址。
fn get_ipv6_via_ipconfig() -> Vec<IpItem> {
    if !cfg!(windows) {
        return Vec::new();
    }

    let output = match Command::new("ipconfig")
        .arg("/all")
        .output()
        .map_err(|e| {
            log::warn!("执行 ipconfig 失败: {e}");
            e
        }) {
        Ok(o) if o.status.success() => o.stdout,
        Ok(o) => {
            log::warn!("执行 ipconfig 失败: 退出码 {}", o.status);
            return Vec::new();
        }
        Err(_) => return Vec::new(),
    };

    let text = String::from_utf8_lossy(&output);

    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in text.lines() {
        let line_lower = line.to_lowercase();
        if !line_lower.contains("ipv6") {
            continue;
        }
        // 取最后 ": " 之后的地址部分
        let value = match line.rsplit_once(": ") {
            Some((_, v)) => v.trim(),
            None => continue,
        };
        // 去掉末尾 (Preferred)/(Deprecated)/(首选) 等状态后缀
        let value = strip_status_suffix(value);
        // 去掉 %zone
        let value = strip_zone(value).trim();

        if let Some(addr) = parse_ipv6(value) {
            if seen.insert(addr.clone()) {
                results.push(IpItem {
                    interface: "N/A".to_string(),
                    address: addr.clone(),
                    scope: scope_for(&addr),
                });
            }
        }
    }

    if results.is_empty() {
        log::debug!("ipconfig 未获取到 IPv6 地址");
    } else {
        log::debug!("ipconfig 获取到 {} 个 IPv6 地址", results.len());
    }
    results
}

/// 去掉 " (Preferred)" / " (Deprecated)" / " (首选)" 等尾部括号后缀。
fn strip_status_suffix(s: &str) -> &str {
    let trimmed = s.trim_end();
    if let Some(open) = trimmed.rfind('(') {
        // 仅当括号位于字符串末尾时去掉
        if trimmed[open..].ends_with(')') {
            return trimmed[..open].trim_end();
        }
    }
    trimmed
}

/// 通过 socket 主机名解析获取 IPv6 地址（备用方法）。
fn get_ipv6_via_socket() -> Vec<IpItem> {
    let mut results = Vec::new();
    let hostname = match hostname::get() {
        Ok(h) => format!("{}", h),
        Err(_) => return results,
    };

    let mut seen = std::collections::HashSet::new();
    let addrs: Vec<SocketAddr> = match (hostname.as_str(), 0u16).to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(_) => return results,
    };

    for a in addrs {
        if let SocketAddr::V6(v6) = a {
            let addr = strip_zone(&v6.ip().to_string()).to_string();
            if seen.insert(addr.clone()) {
                results.push(IpItem {
                    interface: "N/A".to_string(),
                    address: addr.clone(),
                    scope: scope_for(&addr),
                });
            }
        }
    }
    results
}

/// 获取本机所有 IPv6 地址并过滤。
pub fn get_ipv6_addresses(
    exclude_link_local: bool,
    exclude_loopback: bool,
    address_suffix: Option<&str>,
) -> Vec<IpItem> {
    let mut all: Vec<IpItem> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    if cfg!(windows) {
        for item in get_ipv6_via_ipconfig() {
            if seen.insert(item.address.clone()) {
                all.push(item);
            }
        }
    }
    for item in get_ipv6_via_socket() {
        if seen.insert(item.address.clone()) {
            all.push(item);
        }
    }

    if all.is_empty() {
        log::warn!("所有方法均未获取到 IPv6 地址。");
        return Vec::new();
    }

    let mut results = Vec::new();
    for item in all {
        let addr = &item.address;
        if exclude_link_local && is_link_local(addr) {
            continue;
        }
        if exclude_loopback && is_loopback(addr) {
            continue;
        }
        if let Some(suffix) = address_suffix {
            if !suffix.is_empty() && !addr.to_lowercase().ends_with(&suffix.to_lowercase()) {
                continue;
            }
        }
        results.push(item);
    }

    results
}

/// 根据配置文件的 filter 段获取过滤后的 IPv6 地址列表。
pub fn get_filtered_addrs(cfg: &Config) -> Vec<IpItem> {
    let suffix = if cfg.filter.address_suffix.is_empty() {
        None
    } else {
        Some(cfg.filter.address_suffix.as_str())
    };
    get_ipv6_addresses(
        cfg.filter.exclude_link_local,
        cfg.filter.exclude_loopback,
        suffix,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_status_suffix_works() {
        assert_eq!(strip_status_suffix("fe80::1%31(Preferred)"), "fe80::1%31");
        assert_eq!(strip_status_suffix("240e::1008(Deprecated)"), "240e::1008");
        assert_eq!(strip_status_suffix("240e::1008"), "240e::1008");
    }

    #[test]
    fn strip_zone_works() {
        assert_eq!(strip_zone("fe80::1%31"), "fe80::1");
        assert_eq!(strip_zone("240e::1"), "240e::1");
    }

    #[test]
    fn parse_valid_and_invalid_ipv6() {
        assert!(parse_ipv6("240e::1").is_some());
        assert!(parse_ipv6("::1").is_some());
        assert!(parse_ipv6("fe80::e2a6:51a8:f69e:9e9a").is_some());
        assert!(parse_ipv6("240e:3a4:4add:74b0::1008").is_some());
        assert!(parse_ipv6("not-an-ip").is_none());
        assert!(parse_ipv6("192.168.1.1").is_none());
        assert!(parse_ipv6("").is_none());
    }

    #[test]
    fn scope_checks() {
        assert!(is_link_local("fe80::1"));
        assert!(!is_link_local("240e::1"));
        assert!(is_loopback("::1"));
        assert!(!is_loopback("::2"));
        assert!(is_global("240e::1"));
        assert!(is_global("3000::1"));
        assert!(!is_global("fe80::1"));
    }
}
