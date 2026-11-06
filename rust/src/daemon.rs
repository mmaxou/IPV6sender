//! 守护进程控制器（对应 Python 版 DaemonController）。
//!
//! 支持在独立线程中运行，可安全停止。

use crate::config::Config;
use crate::scheduler::Scheduler;
use crate::{bark, ipv6, state};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// 守护进程控制器。
pub struct DaemonController {
    cfg: Config,
    stop: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl DaemonController {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            stop: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// 启动守护线程，返回是否成功启动。
    pub fn start(&mut self) -> bool {
        if self.is_running() {
            log::warn!("守护进程已在运行中。");
            return false;
        }

        self.stop.store(false, Ordering::SeqCst);
        self.running.store(true, Ordering::SeqCst);

        let stop = Arc::clone(&self.stop);
        let running = Arc::clone(&self.running);
        let cfg = self.cfg.clone();

        let handle = thread::Builder::new()
            .name("ipsender-daemon".to_string())
            .spawn(move || {
                run_daemon_loop(&cfg, stop);
                running.store(false, Ordering::SeqCst);
            })
            .expect("无法创建守护线程");

        self.handle = Some(handle);
        true
    }

    /// 停止守护进程（非阻塞，不等待线程退出）。
    pub fn stop(&mut self) {
        if !self.is_running() {
            return;
        }
        self.stop.store(true, Ordering::SeqCst);
        self.handle = None;
        log::info!("守护进程已停止。");
    }
}

fn run_daemon_loop(cfg: &Config, stop: Arc<AtomicBool>) {
    let sched_cfg = &cfg.schedule;
    let notif_cfg = &cfg.notification;
    let check_interval = std::cmp::max(sched_cfg.check_interval_minutes, 1);
    let daily_enabled = sched_cfg.daily_enabled;
    let daily_time = sched_cfg.daily_time.clone();
    let change_detection = sched_cfg.change_detection;
    let state_path = crate::default_state_path();

    let last_ip = state::load_last_ip(&state_path);
    log::info!(
        "守护进程启动。上次记录的 IP: {}",
        if last_ip.is_empty() {
            "(无)".to_string()
        } else {
            last_ip.clone()
        }
    );

    let state_last = Rc::new(RefCell::new(last_ip));

    let mut scheduler = Scheduler::new();

    if change_detection {
        let state_last = Rc::clone(&state_last);
        let path = state_path.clone();
        let ncfg = notif_cfg.clone();
        let cfg = cfg.clone();
        scheduler.every_minutes(
            check_interval,
            Box::new(move || check_ip_change(&cfg, &state_last, &path, &ncfg)),
        );
        log::info!("已启用 IP 变化检测，每 {} 分钟检查一次。", check_interval);
    }

    if daily_enabled {
        let state_last = Rc::clone(&state_last);
        let path = state_path.clone();
        let ncfg = notif_cfg.clone();
        let cfg = cfg.clone();
        match scheduler.daily_at(
            &daily_time,
            Box::new(move || daily_report(&cfg, &state_last, &path, &ncfg)),
        ) {
            Ok(()) => log::info!("已启用每日定时报告，每天 {} 发送。", daily_time),
            Err(e) => log::warn!("{e}"),
        }
    }

    log::info!("守护进程已启动。");

    // 启动时立即检测一次 IP
    {
        let addrs = ipv6::get_filtered_addrs(cfg);
        let current_ip = addrs.first().map(|a| a.address.clone()).unwrap_or_default();
        let mut st = state_last.borrow_mut();
        if !current_ip.is_empty() && st.is_empty() {
            log::info!("首次运行，记录当前 IP: {}", current_ip);
            *st = current_ip.clone();
            drop(st);
            state::save_last_ip(&state_path, &current_ip);
        } else if !current_ip.is_empty() && current_ip != *st {
            log::info!("启动时检测到 IP 与上次记录不同，立即推送。");
            drop(st);
            check_ip_change(cfg, &state_last, &state_path, notif_cfg);
        } else {
            log::info!(
                "当前 IP 与上次记录一致: {}",
                if current_ip.is_empty() {
                    "(无)".to_string()
                } else {
                    current_ip
                }
            );
        }
    }

    // 主循环
    while !stop.load(Ordering::SeqCst) {
        scheduler.run_pending();
        thread::sleep(Duration::from_secs(30));
    }

    log::info!("守护进程主循环已退出。");
}

fn check_ip_change(
    cfg: &Config,
    state_last: &Rc<RefCell<String>>,
    state_path: &std::path::Path,
    notif: &crate::config::NotificationConfig,
) {
    let addrs = ipv6::get_filtered_addrs(cfg);
    let current_ip = addrs.first().map(|a| a.address.clone()).unwrap_or_default();

    let mut st = state_last.borrow_mut();
    if current_ip == *st {
        log::debug!("IP 未变化: {}", if current_ip.is_empty() { "(无)".to_string() } else { current_ip.clone() });
        return;
    }

    let old = if st.is_empty() {
        "(无)".to_string()
    } else {
        st.clone()
    };
    log::info!(
        "检测到 IP 变化: {} -> {}",
        old,
        if current_ip.is_empty() {
            "(无)".to_string()
        } else {
            current_ip.clone()
        }
    );

    if !current_ip.is_empty() {
        let title = if notif.title_changed.is_empty() {
            "[IPsender] IPv6地址已变化!".to_string()
        } else {
            notif.title_changed.clone()
        };
        bark::send_notification(cfg, &addrs, Some(&title));
    } else {
        log::warn!("当前未检测到 IPv6 地址，跳过推送。");
    }

    *st = current_ip.clone();
    drop(st);
    state::save_last_ip(state_path, &current_ip);
}

fn daily_report(
    cfg: &Config,
    state_last: &Rc<RefCell<String>>,
    state_path: &std::path::Path,
    notif: &crate::config::NotificationConfig,
) {
    log::info!("执行每日定时报告...");
    let addrs = ipv6::get_filtered_addrs(cfg);
    let title = if notif.title_daily.is_empty() {
        "[IPsender] 每日IPv6地址报告".to_string()
    } else {
        notif.title_daily.clone()
    };
    bark::send_notification(cfg, &addrs, Some(&title));

    let current_ip = addrs.first().map(|a| a.address.clone()).unwrap_or_default();
    let mut st = state_last.borrow_mut();
    *st = current_ip.clone();
    drop(st);
    state::save_last_ip(state_path, &current_ip);
}

/// 命令行方式的守护进程入口（阻塞运行，直到收到 Ctrl+C 等中断）。
pub fn run_daemon(cfg: Config) {
    let mut ctrl = DaemonController::new(cfg);
    ctrl.start();
    while ctrl.is_running() {
        thread::sleep(Duration::from_secs(1));
    }
}
