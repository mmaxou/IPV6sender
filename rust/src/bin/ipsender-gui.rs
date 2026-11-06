#![windows_subsystem = "windows"]

//! IPsender 图形界面版（对应 Python 版 gui.py）。
//!
//! 功能：
//!   - 开始/停止检测（守护进程）
//!   - 开机自启开关（写入 Windows 注册表）
//!   - 最小化到系统托盘
//!   - 实时日志显示
//!
//! 架构要点（线程安全）：
//!   核心逻辑运行在独立线程；所有跨线程消息通过 mpsc 通道传递，
//!   主线程（egui/eframe）每帧轮询通道并刷新界面。

use eframe::egui::{self, Color32, RichText};
use ipsender::bark;
use ipsender::base_dir;
use ipsender::config::{load_config, Config};
use ipsender::daemon::DaemonController;
use ipsender::ipv6;
use ipsender::default_config_path;
use log::LevelFilter;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

// ---------------------------------------------------------------------------
// 跨线程消息
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum GuiMsg {
    Log(String, String), // (消息, 颜色)
    Ip(String),          // IP 地址更新
    Show,                // 托盘"显示窗口"
    Quit,                // 托盘/程序"退出"
}

static MSG_TX: OnceLock<Sender<GuiMsg>> = OnceLock::new();

fn level_color(level: log::Level) -> &'static str {
    match level {
        log::Level::Debug => "#888888",
        log::Level::Info => "#d4d4d4",
        log::Level::Warn => "#e0a030",
        log::Level::Error => "#f06060",
        log::Level::Trace => "#808080",
    }
}

fn parse_color(hex: &str) -> Color32 {
    let h = hex.trim_start_matches('#');
    if h.len() == 6 {
        if let Ok(v) = u32::from_str_radix(h, 16) {
            return Color32::from_rgb(
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            );
        }
    }
    Color32::from_gray(212)
}

/// 日志处理器：把日志转发到 GUI 通道（线程安全）。
struct QueueLogger;

impl log::Log for QueueLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }
    fn log(&self, record: &log::Record) {
        if let Some(tx) = MSG_TX.get() {
            let _ = tx.send(GuiMsg::Log(
                record.args().to_string(),
                level_color(record.level()).to_string(),
            ));
        }
    }
    fn flush(&self) {}
}

// ---------------------------------------------------------------------------
// 开机自启管理（Windows 注册表）
// ---------------------------------------------------------------------------

const APP_NAME: &str = "IPsender";

#[cfg(windows)]
fn autostart_enabled() -> bool {
    use winreg::enums::*;
    use winreg::RegKey;
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_READ)
    {
        matches!(key.get_value::<String, _>(APP_NAME), Ok(_))
    } else {
        false
    }
}

#[cfg(windows)]
fn autostart_set(enable: bool) -> bool {
    use winreg::enums::*;
    use winreg::RegKey;
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        KEY_SET_VALUE,
    ) {
        Ok(k) => k,
        Err(_) => return false,
    };
    if enable {
        match std::env::current_exe() {
            Ok(exe) => {
                let cmd = format!("\"{}\"", exe.display());
                key.set_value(APP_NAME, &cmd).is_ok()
            }
            Err(_) => false,
        }
    } else {
        match key.delete_value(APP_NAME) {
            Ok(_) => true,
            Err(_) => false, // 已不存在也视为成功
        }
    }
}

#[cfg(not(windows))]
fn autostart_enabled() -> bool {
    false
}

#[cfg(not(windows))]
fn autostart_set(_enable: bool) -> bool {
    false
}

// ---------------------------------------------------------------------------
// 系统托盘（Windows）
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn run_tray_thread(tx: Sender<GuiMsg>) {
    thread::spawn(move || {
        // Windows 上创建托盘图标前必须先初始化 COM，否则图标不会出现
        let _com = match tray_icon::util::initialize_com() {
            Ok(g) => g,
            Err(_) => return,
        };
        use tray_icon::menu::{Menu, MenuItem};
        use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

        // 生成 64x64 蓝色圆形图标（RGBA）
        let mut rgba = vec![0u8; 64 * 64 * 4];
        let (cx, cy, cr) = (31.5f32, 31.5f32, 29.0f32);
        for y in 0..64 {
            for x in 0..64 {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                let idx = (y * 64 + x) * 4;
                if d <= cr {
                    rgba[idx] = 41;
                    rgba[idx + 1] = 128;
                    rgba[idx + 2] = 185;
                    rgba[idx + 3] = 255;
                }
            }
        }
        let icon = match Icon::from_rgba(rgba, 64, 64) {
            Ok(i) => i,
            Err(_) => return,
        };

        let show = MenuItem::new("显示窗口", true, None);
        let quit = MenuItem::new("退出", true, None);
        let menu = Menu::new();
        if menu.append_items(&[&show, &quit]).is_err() {
            return;
        }
        if TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_tooltip("IPsender")
            .build()
            .is_err()
        {
            return;
        }

        loop {
            while let Ok(ev) = tray_icon::menu::MenuEvent::receiver().try_recv() {
                if ev.id == quit.id() {
                    let _ = tx.send(GuiMsg::Quit);
                    return;
                }
                if ev.id == show.id() {
                    let _ = tx.send(GuiMsg::Show);
                }
            }
            while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = ev
                {
                    let _ = tx.send(GuiMsg::Show);
                }
            }
            thread::sleep(Duration::from_millis(200));
        }
    });
}

#[cfg(not(windows))]
fn run_tray_thread(_tx: Sender<GuiMsg>) {}

// ---------------------------------------------------------------------------
// 中文字体加载（egui 默认字体不含 CJK 字形，需回退字体）
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn load_cjk_font() -> Option<egui::FontData> {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\Deng.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyhl.ttc",
    ];
    for p in CANDIDATES {
        if let Ok(bytes) = std::fs::read(p) {
            // 仅接受能被解析为合法字体的文件（.ttc 集合 ab_glyph 未必支持）
            if ab_glyph::FontArc::try_from_vec(bytes.clone()).is_ok() {
                return Some(egui::FontData::from_owned(bytes));
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn load_cjk_font() -> Option<egui::FontData> {
    None
}

fn setup_fonts(cc: &eframe::CreationContext) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(fd) = load_cjk_font() {
        fonts.font_data.insert("cjk-fallback".to_owned(), fd);
        for fam in [
            egui::FontFamily::Proportional,
            egui::FontFamily::Monospace,
        ] {
            fonts
                .families
                .entry(fam)
                .or_default()
                .push("cjk-fallback".to_owned());
        }
    }
    cc.egui_ctx.set_fonts(fonts);
}

// ---------------------------------------------------------------------------
// 配置加载：默认路径不存在时自动从 config.example.json 播种
// ---------------------------------------------------------------------------

fn find_example_config() -> Option<std::path::PathBuf> {
    let candidates = [
        base_dir().join("config.example.json"),
        std::env::current_dir().unwrap_or_default().join("config.example.json"),
    ];
    for c in candidates {
        if c.exists() {
            return Some(c);
        }
    }
    None
}

fn load_or_seed_config() -> Option<Config> {
    let path = default_config_path();
    if let Ok(c) = load_config(&path) {
        return Some(c);
    }
    if let Some(example) = find_example_config() {
        if !path.exists() {
            if std::fs::copy(&example, &path).is_ok() {
                log::info!(
                    "未找到 config.json，已从 {} 生成默认配置 {}",
                    example.display(),
                    path.display()
                );
                if let Ok(c) = load_config(&path) {
                    return Some(c);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 主窗口
// ---------------------------------------------------------------------------

struct App {
    cfg: Option<Config>,
    daemon: Option<DaemonController>,
    rx: Receiver<GuiMsg>,
    log_lines: Vec<(String, String)>,
    current_ip: String,
    status: String,
    autostart_on: bool,
    quit_requested: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext, rx: Receiver<GuiMsg>) -> Self {
        setup_fonts(cc);

        let cfg = load_or_seed_config();
        let autostart_on = autostart_enabled();

        if cfg.is_none() {
            log::error!("配置加载失败，请检查 config.json。");
        }

        let app = Self {
            cfg,
            daemon: None,
            rx,
            log_lines: Vec::new(),
            current_ip: "--".to_string(),
            status: "状态: 未启动".to_string(),
            autostart_on,
            quit_requested: false,
        };

        // 在子线程获取初始 IP，避免阻塞主线程
        let cfg = app.cfg.clone();
        let tx = MSG_TX.get().cloned();
        thread::spawn(move || {
            if let Some(cfg) = cfg {
                let addrs = ipv6::get_filtered_addrs(&cfg);
                if let Some(first) = addrs.first() {
                    let ip = first.address.clone();
                    if let Some(tx) = &tx {
                        let _ = tx.send(GuiMsg::Ip(ip.clone()));
                    }
                    log::info!("检测到 IPv6 地址: {}", ip);
                } else {
                    log::warn!("未检测到 IPv6 地址。");
                }
            }
        });

        app
    }

    fn toggle_daemon(&mut self) {
        if let Some(d) = &mut self.daemon {
            if d.is_running() {
                d.stop();
                self.status = "状态: 已停止".to_string();
                log::info!("检测已停止。");
                return;
            }
        }
        if let Some(cfg) = self.cfg.clone() {
            let mut d = DaemonController::new(cfg);
            if d.start() {
                self.daemon = Some(d);
                self.status = "状态: 运行中".to_string();
                log::info!("检测已启动。");
            }
        } else {
            log::error!("配置文件未加载，无法启动。");
        }
    }

    fn send_once(&self) {
        if let Some(cfg) = self.cfg.clone() {
            let tx = MSG_TX.get().cloned();
            thread::spawn(move || {
                log::info!("手动触发推送...");
                let addrs = ipv6::get_filtered_addrs(&cfg);
                if let Some(first) = addrs.first() {
                    if let Some(tx) = &tx {
                        let _ = tx.send(GuiMsg::Ip(first.address.clone()));
                    }
                }
                bark::send_notification(&cfg, &addrs, None);
            });
        } else {
            log::error!("配置文件未加载。");
        }
    }

    fn toggle_autostart(&mut self) {
        let enable = !self.autostart_on;
        if autostart_set(enable) {
            self.autostart_on = enable;
            if enable {
                log::info!("已设置开机自启。");
            } else {
                log::info!("已取消开机自启。");
            }
        }
    }

    fn hide_to_tray(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        log::info!("已最小化到系统托盘。");
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 每帧轮询消息队列
        let mut actions = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            actions.push(msg);
        }
        for msg in actions {
            match msg {
                GuiMsg::Log(text, color) => {
                    self.log_lines.push((text, color));
                    // 限制日志条数，防止长期运行内存无限增长
                    const MAX_LOG: usize = 800;
                    if self.log_lines.len() > MAX_LOG {
                        let excess = self.log_lines.len() - MAX_LOG;
                        self.log_lines.drain(..excess);
                    }
                }
                GuiMsg::Ip(ip) => self.current_ip = ip,
                GuiMsg::Show => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                }
                GuiMsg::Quit => self.quit_requested = true,
            }
        }

        if self.quit_requested {
            log::info!("退出程序");
            if let Some(d) = &mut self.daemon {
                d.stop();
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // ---- 状态栏 ----
        egui::TopBottomPanel::top("status_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).size(14.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("当前 IP: {}", self.current_ip)).monospace());
                });
            });
            ui.add_space(4.0);
        });

        // ---- 按钮栏 ----
        egui::TopBottomPanel::top("buttons").show(ctx, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let running = self
                    .daemon
                    .as_ref()
                    .map(|d| d.is_running())
                    .unwrap_or(false);
                let start_label = if running { "停止检测" } else { "开始检测" };
                if ui.button(start_label).clicked() {
                    self.toggle_daemon();
                }
                if ui.button("立即推送").clicked() {
                    self.send_once();
                }
                let autostart_label = if self.autostart_on {
                    "开机自启: 开"
                } else {
                    "开机自启: 关"
                };
                if ui.button(autostart_label).clicked() {
                    self.toggle_autostart();
                }
                if cfg!(windows) {
                    if ui.button("最小化到托盘").clicked() {
                        self.hide_to_tray(ctx);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("退出程序").clicked() {
                        self.quit_requested = true;
                    }
                    if ui.button("清空日志").clicked() {
                        self.log_lines.clear();
                    }
                });
            });
            ui.add_space(2.0);
            ui.separator();
        });

        // ---- 日志区 ----
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("运行日志");
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (text, color) in &self.log_lines {
                        ui.label(RichText::new(text).color(parse_color(color)).monospace());
                    }
                });
            ui.add_space(4.0);
            ui.label(
                RichText::new("IPsender v1.0  |  Bark 推送  |  托盘可恢复窗口")
                    .size(11.0)
                    .weak(),
            );
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(d) = &mut self.daemon {
            d.stop();
        }
    }
}

/// 打开（或创建）位于可执行文件旁的错误日志文件。
fn error_log_file() -> Option<std::fs::File> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let path = dir.join("ipsender-gui-error.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
}

/// 安装 panic 处理器：把崩溃信息写入日志文件，便于排查无控制台的"闪退"。
fn install_panic_log() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(mut f) = error_log_file() {
            use std::io::Write;
            let _ = writeln!(f, "[panic] {}", info);
        }
        prev(info);
    }));
}

/// 记录启动阶段的错误。
fn log_startup_error(msg: &str) {
    if let Some(mut f) = error_log_file() {
        use std::io::Write;
        let _ = writeln!(f, "[startup] {}", msg);
    }
}

fn main() {
    install_panic_log();

    let (tx, rx) = channel::<GuiMsg>();
    let _ = MSG_TX.set(tx.clone());

    let _ = log::set_boxed_logger(Box::new(QueueLogger));
    log::set_max_level(LevelFilter::Debug);

    run_tray_thread(tx);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([660.0, 480.0])
            .with_min_inner_size([540.0, 380.0])
            .with_title("IPsender - IPv6 地址推送"),
        ..Default::default()
    };

    // eframe 0.27：创建闭包直接返回 Box<dyn App>（不是 Result）
    let creator = Box::new(move |_cc: &eframe::CreationContext| -> Box<dyn eframe::App> {
        Box::new(App::new(_cc, rx))
    });

    if let Err(e) = eframe::run_native("IPsender", options, creator) {
        let msg = format!("GUI 启动失败: {e}");
        log_startup_error(&msg);
        eprintln!("{msg}");
        std::process::exit(1);
    }
}
