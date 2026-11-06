//! 轻量级定时调度器（对应 Python 版 SimpleScheduler）。

use chrono::{Local, TimeZone};
use std::time::{Duration, Instant};

enum JobKind {
    /// 每 N 秒执行一次。
    Interval {
        interval: Duration,
        next: Instant,
    },
    /// 每天 HH:MM 执行一次。
    Daily {
        hour: u32,
        minute: u32,
        next: i64, // 下次运行的 Unix 时间戳
    },
}

struct Job {
    kind: JobKind,
    f: Box<dyn FnMut()>,
}

/// 极简定时调度器，仅用于单线程守护循环。
pub struct Scheduler {
    jobs: Vec<Job>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { jobs: Vec::new() }
    }

    /// 注册一个每 N 分钟执行一次的任务。
    pub fn every_minutes(&mut self, interval_minutes: u64, f: Box<dyn FnMut()>) {
        let interval = Duration::from_secs(interval_minutes.saturating_mul(60));
        self.jobs.push(Job {
            kind: JobKind::Interval {
                interval,
                next: Instant::now() + interval,
            },
            f,
        });
    }

    /// 注册一个每天指定时间（"HH:MM"）执行的任务。
    pub fn daily_at(&mut self, time_str: &str, f: Box<dyn FnMut()>) -> Result<(), String> {
        let parts: Vec<&str> = time_str.split(':').collect();
        if parts.len() != 2 {
            return Err(format!("无效的每日时间: {time_str}"));
        }
        let hour: u32 = parts[0].parse().map_err(|_| format!("无效的每日时间: {time_str}"))?;
        let minute: u32 = parts[1].parse().map_err(|_| format!("无效的每日时间: {time_str}"))?;
        if hour > 23 || minute > 59 {
            return Err(format!("无效的每日时间: {time_str}"));
        }

        let now = Local::now();
        let mut target = today_at(now, hour, minute);
        if now >= target {
            target += chrono::Duration::days(1);
        }

        self.jobs.push(Job {
            kind: JobKind::Daily {
                hour,
                minute,
                next: target.timestamp(),
            },
            f,
        });
        Ok(())
    }

    /// 检查并执行到期的任务。
    pub fn run_pending(&mut self) {
        let now_mono = Instant::now();
        let now = Local::now();

        for job in &mut self.jobs {
            match &mut job.kind {
                JobKind::Interval { interval, next } => {
                    if now_mono >= *next {
                        (job.f)();
                        *next = Instant::now() + *interval;
                    }
                }
                JobKind::Daily { hour, minute, next } => {
                    let target_today = today_at(now, *hour, *minute);
                    if now >= target_today && *next <= now.timestamp() {
                        (job.f)();
                        *next = (target_today + chrono::Duration::days(1)).timestamp();
                    }
                }
            }
        }
    }

    pub fn has_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }
}

/// 构造今天指定时刻的本地时间。
fn today_at(now: chrono::DateTime<Local>, hour: u32, minute: u32) -> chrono::DateTime<Local> {
    let naive = now
        .date_naive()
        .and_hms_opt(hour, minute, 0)
        .unwrap_or_else(|| now.naive_local());
    Local
        .from_local_datetime(&naive)
        .earliest()
        .unwrap_or(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn daily_at_rejects_invalid_time() {
        let mut s = Scheduler::new();
        assert!(s.daily_at("abc", Box::new(|| {})).is_err());
        assert!(s.daily_at("24:00", Box::new(|| {})).is_err());
        assert!(s.daily_at("08", Box::new(|| {})).is_err());
        assert!(s.daily_at("08:30", Box::new(|| {})).is_ok());
    }

    #[test]
    fn interval_job_registers() {
        let mut s = Scheduler::new();
        s.every_minutes(1, Box::new(|| {}));
        assert!(s.has_jobs());
    }

    #[test]
    fn today_at_keeps_clock_time() {
        let now = Local::now();
        let target = today_at(now, 8, 0);
        assert_eq!(target.hour(), 8);
        assert_eq!(target.minute(), 0);
    }
}
