//! 配置加载与校验（对应 Python 版 load_config）。
//!
//! 配置 JSON 结构与 Python 版完全一致，缺省字段使用与 Python 版相同的默认值。

use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BarkConfig {
    pub server: String,
    pub key: String,
    pub timeout: u64,
}

impl Default for BarkConfig {
    fn default() -> Self {
        Self {
            server: String::new(),
            key: String::new(),
            timeout: 15,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NotificationConfig {
    pub title: String,
    pub title_daily: String,
    pub title_changed: String,
    pub group: String,
    pub sound: String,
    pub icon: String,
    pub level: String,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            title: "[IPsender] 本机IPv6地址更新".to_string(),
            title_daily: "[IPsender] 每日IPv6地址报告".to_string(),
            title_changed: "[IPsender] IPv6地址已变化!".to_string(),
            group: String::new(),
            sound: String::new(),
            icon: String::new(),
            level: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScheduleConfig {
    pub enabled: bool,
    pub check_interval_minutes: u64,
    pub daily_enabled: bool,
    pub daily_time: String,
    pub change_detection: bool,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            check_interval_minutes: 5,
            daily_enabled: true,
            daily_time: "08:00".to_string(),
            change_detection: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FilterConfig {
    pub exclude_link_local: bool,
    pub exclude_loopback: bool,
    pub interfaces: Vec<String>,
    pub address_suffix: String,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            exclude_link_local: true,
            exclude_loopback: true,
            interfaces: Vec::new(),
            address_suffix: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub bark: BarkConfig,
    pub notification: NotificationConfig,
    pub schedule: ScheduleConfig,
    pub filter: FilterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bark: BarkConfig::default(),
            notification: NotificationConfig::default(),
            schedule: ScheduleConfig::default(),
            filter: FilterConfig::default(),
        }
    }
}

/// 从 JSON 文件加载配置并做基本校验。
pub fn load_config<P: AsRef<Path>>(config_path: P) -> Result<Config, String> {
    let path = config_path.as_ref();
    if !path.exists() {
        log::error!("配置文件不存在: {}", path.display());
        log::info!("请复制 config.example.json 为 config.json 并修改其中的配置。");
        return Err(format!("配置文件不存在: {}", path.display()));
    }

    let text = fs::read_to_string(path).map_err(|e| format!("读取配置文件失败: {e}"))?;
    let cfg: Config = serde_json::from_str(&text).map_err(|e| format!("配置文件解析失败: {e}"))?;

    if cfg.bark.server.trim().is_empty() {
        return Err("配置文件中 bark.server 不能为空。".to_string());
    }
    if cfg.bark.key.trim().is_empty() {
        return Err("配置文件中 bark.key 不能为空。".to_string());
    }

    log::debug!("配置加载完成: {}", path.display());
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_python() {
        let c = Config::default();
        assert_eq!(c.bark.timeout, 15);
        assert_eq!(c.schedule.check_interval_minutes, 5);
        assert_eq!(c.schedule.daily_time, "08:00");
        assert!(c.schedule.change_detection);
        assert!(c.filter.exclude_link_local);
        assert!(c.filter.exclude_loopback);
        assert_eq!(c.notification.title, "[IPsender] 本机IPv6地址更新");
    }

    fn temp_config_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "ipsender_cfg_{}_{}.json",
            tag,
            std::process::id()
        ))
    }

    #[test]
    fn load_config_from_file_ok() {
        let path = temp_config_path("ok");
        let json = r#"{
            "bark": {"server": "https://api.day.app", "key": "abc", "timeout": 7},
            "notification": {"title": "T", "group": "G"},
            "schedule": {"check_interval_minutes": 10, "daily_time": "09:30"},
            "filter": {"exclude_link_local": false, "address_suffix": "1008"}
        }"#;
        std::fs::write(&path, json).unwrap();
        let cfg = load_config(&path).expect("should parse");
        let _ = std::fs::remove_file(&path);

        assert_eq!(cfg.bark.server, "https://api.day.app");
        assert_eq!(cfg.bark.key, "abc");
        assert_eq!(cfg.bark.timeout, 7);
        assert_eq!(cfg.notification.title, "T");
        assert_eq!(cfg.notification.group, "G");
        assert_eq!(cfg.schedule.check_interval_minutes, 10);
        assert_eq!(cfg.schedule.daily_time, "09:30");
        assert!(!cfg.filter.exclude_link_local);
        assert_eq!(cfg.filter.address_suffix, "1008");
    }

    #[test]
    fn load_config_rejects_missing_key() {
        let path = temp_config_path("missing_key");
        let json = r#"{"bark": {"server": "https://api.day.app", "key": ""}}"#;
        std::fs::write(&path, json).unwrap();
        let res = load_config(&path);
        let _ = std::fs::remove_file(&path);
        assert!(res.is_err());
    }

    #[test]
    fn load_config_rejects_missing_file() {
        let path = temp_config_path("nonexistent");
        let _ = std::fs::remove_file(&path); // ensure不存在
        assert!(load_config(&path).is_err());
    }
}
