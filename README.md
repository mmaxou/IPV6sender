# IPsender

自动获取本机 IPv6 地址并通过 [Bark](https://bark.day.app) 推送通知到 iPhone / iPad，**Rust 重写版**（原 Python 版已移除）。

## 功能特性

- 🔍 **自动检测 IPv6 地址** —— 解析 `ipconfig /all` 输出（Windows，最可靠），另以主机名解析作为回退；支持过滤链路本地地址 (`fe80::/10`)、回环地址 (`::1`)，以及按地址后缀过滤
- 📱 **Bark 推送** —— 通过 Bark API 推送，支持自定义标题、分组、铃声、图标
- ⏰ **每日定时报告** —— 每天在指定时间（如早 8 点）自动发送一次 IPv6 地址通知
- 🔄 **IP 变化检测** —— 定期轮询，当地址变化时立即推送
- 💾 **状态持久化** —— 通过 `state.json` 记录上次已知 IP，重启后自动恢复
- 🖥️ **两种界面** —— 命令行版（跨平台）+ 图形界面版（Windows）

## 项目结构

```
IPsender/
├── rust/                         # Rust 工程
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs            # 核心库入口
│   │   ├── config.rs         # 配置加载与校验
│   │   ├── state.rs          # state.json 状态持久化
│   │   ├── ipv6.rs           # IPv6 地址获取与过滤
│   │   ├── bark.rs           # Bark 推送
│   │   ├── scheduler.rs      # 定时调度器（间隔 + 每日）
│   │   ├── daemon.rs         # 守护进程控制器
│   │   └── bin/
│   │       ├── ipsender.rs       # CLI 入口
│   │       └── ipsender-gui.rs   # GUI 入口（eframe）
│   └── (target/ 构建产物，gitignore 忽略)
├── .github/workflows/build.yml  # GitHub Actions 自动构建
├── config.example.json           # 配置文件模板
├── config.json                   # 实际配置（自行创建，不纳入版本管理）
├── state.json                    # 运行时状态（自动生成）
└── README.md
```

## 编译

推荐使用 **GitHub Actions**：推送仓库后自动构建 CLI（Windows/Linux/macOS）与 GUI（Windows）并上传产物（见 `.github/workflows/build.yml`）。

本地编译（需 Rust 工具链）：

```bash
cd rust

# CLI（不依赖 GUI 重依赖，构建快）
cargo build --release --no-default-features --bin ipsender

# GUI（Windows）
cargo build --release --features gui --bin ipsender-gui

# 运行测试
cargo test
```

## 使用

### 1. 创建配置

复制模板并修改 Bark 配置：

```bash
cp config.example.json config.json
```

CLI 默认从「可执行文件所在目录」读取 `config.json`，也可用 `--config` 显式指定。

### 2. 命令行版

```bash
# 单次运行（获取地址并推送一次）
ipsender --config config.json

# 守护进程模式（每日定时 + IP 变化检测）
ipsender --config config.json --daemon

# 仅打印 IPv6 地址（测试，不发推送）
ipsender --print-only

# 启用调试日志
ipsender --config config.json --verbose
```

### 3. 图形界面版（Windows）

```bash
ipsender-gui
```

功能：开始/停止检测（守护进程）、立即推送、开机自启开关、最小化到托盘、清空日志、退出程序。

## 配置项说明

配置文件 JSON 结构与字段含义：

| 字段 | 说明 |
|------|------|
| `bark.server` | Bark 服务器地址（官方: `https://api.day.app`，也可自建） |
| `bark.key` | Bark 推送 Key（在 Bark App 中复制获取） |
| `bark.timeout` | HTTP 请求超时（秒），默认 15 |
| `notification.title` | 默认推送标题 |
| `notification.title_daily` | 每日定时报告标题 |
| `notification.title_changed` | IP 变化标题 |
| `notification.group` | 推送分组名称 |
| `notification.sound` / `icon` / `level` | 铃声 / 图标 URL / 通知级别（留空使用默认） |
| `schedule.check_interval_minutes` | IP 变化轮询间隔（分钟），默认 5 |
| `schedule.daily_time` | 每日报告时间（24 小时制，如 `08:00`） |
| `schedule.daily_enabled` / `change_detection` | 每日报告 / IP 变化检测开关 |
| `filter.exclude_link_local` / `exclude_loopback` | 是否排除链路本地 / 回环地址 |
| `filter.address_suffix` | 仅保留以此后缀结尾的地址（留空不过滤） |

## 开机自启动（可选）

- **Windows**：GUI 中点击「开机自启」开关（写入当前用户注册表 `Run` 项）；或用任务计划程序在登录/启动时运行 `ipsender --config <路径> --daemon`。
- **Linux**：systemd 或 crontab 调度 `ipsender --config <路径> --daemon`。

## 许可证

MIT License
