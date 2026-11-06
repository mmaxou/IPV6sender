//! IPsender 命令行版（对应 Python 版 ipv6_sender.py 的入口）。

use ipsender::config::load_config;
use ipsender::ipv6::{get_ipv6_addresses, IpItem};
use ipsender::{default_config_path, run_task};
use log::LevelFilter;
use std::env;

fn format_ipv6_table(addrs: &[IpItem]) -> String {
    if addrs.is_empty() {
        return "(未检测到 IPv6 地址)".to_string();
    }
    let header = format!("{:<45} {}", "IPv6 地址", "范围");
    let mut lines = vec![header.clone(), "-".repeat(header.chars().count())];
    for item in addrs {
        // 用全角空格补齐，保证多列对齐（与 Python 语义一致）
        let mut line = format!("{:<45} {}", item.address, item.scope);
        if item.address.chars().count() < 45 {
            let pad = 45 - item.address.chars().count();
            line = format!("{}{} {}", item.address, " ".repeat(pad), item.scope);
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn print_help() {
    println!("IPsender - 自动获取本机 IPv6 地址并通过 Bark 推送通知");
    println!();
    println!("用法:");
    println!("  ipsender [选项]");
    println!();
    println!("选项:");
    println!("  -c, --config <路径>   配置文件路径 (默认: {}）", default_config_path().display());
    println!("  -d, --daemon           以守护进程模式运行（每日定时 + IP 变化检测）");
    println!("  -p, --print-only        仅获取并打印 IPv6 地址，不发送通知");
    println!("  -v, --verbose           启用调试日志");
    println!("  -h, --help              显示帮助");
}

struct Args {
    config: String,
    daemon: bool,
    print_only: bool,
    verbose: bool,
}

fn parse_args(args: &[String]) -> Args {
    let mut a = Args {
        config: default_config_path().to_string_lossy().into_owned(),
        daemon: false,
        print_only: false,
        verbose: false,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-c" | "--config" => {
                if let Some(v) = args.get(i + 1) {
                    a.config = v.clone();
                    i += 1;
                }
            }
            "-d" | "--daemon" => a.daemon = true,
            "-p" | "--print-only" => a.print_only = true,
            "-v" | "--verbose" => a.verbose = true,
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "-c=" => {}
            other if other.starts_with("--config=") => {
                a.config = other["--config=".len()..].to_string();
            }
            other if other.starts_with("-c=") => {
                a.config = other["-c=".len()..].to_string();
            }
            _ => {}
        }
        i += 1;
    }
    a
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let opts = parse_args(&args);

    let mut builder = env_logger::Builder::new();
    builder
        .filter_level(if opts.verbose {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        })
        .format_timestamp_secs();
    builder.init();

    log::info!("IPsender 启动 (PID: {})", std::process::id());

    if opts.print_only {
        let addrs = get_ipv6_addresses(true, true, None);
        println!();
        println!("{}", format_ipv6_table(&addrs));
        println!();
        return;
    }

    let cfg = match load_config(&opts.config) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("错误: {e}");
            std::process::exit(1);
        }
    };

    if opts.daemon {
        ipsender::daemon::run_daemon(cfg);
    } else {
        run_task(&cfg);
    }
}
