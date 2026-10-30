use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Off = 4,
}

impl LogLevel {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "debug" => LogLevel::Debug,
            "warn" | "warning" => LogLevel::Warn,
            "error" => LogLevel::Error,
            "off" | "none" => LogLevel::Off,
            _ => LogLevel::Info,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warn => "WARN",
            LogLevel::Error => "ERROR",
            LogLevel::Off => "OFF",
        }
    }
}

static FOREGROUND_LOGGING: AtomicBool = AtomicBool::new(false);
static CURRENT_LOG_LEVEL: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

struct LoggerInner {
    log_dir: PathBuf,
    current_date: String,
    file: Option<File>,
    retention_days: u64,
}

static LOGGER_STATE: Mutex<Option<LoggerInner>> = Mutex::new(None);

pub fn default_log_dir() -> PathBuf {
    std::env::var("USERPROFILE")
        .map(|p| PathBuf::from(p).join(".wallpaper-shuffler").join("logs"))
        .unwrap_or_else(|_| PathBuf::from("target").join("logs"))
}

pub fn is_foreground() -> bool {
    FOREGROUND_LOGGING.load(Ordering::Relaxed)
}

pub fn init(foreground: bool, level: LogLevel) {
    FOREGROUND_LOGGING.store(foreground, Ordering::SeqCst);
    CURRENT_LOG_LEVEL.store(level as u8, Ordering::SeqCst);

    let log_dir = default_log_dir();
    let _ = fs::create_dir_all(&log_dir);

    // Initial log retention cleanup (keep 7 days)
    prune_old_logs(&log_dir, 7);

    let mut guard = LOGGER_STATE.lock().unwrap();
    *guard = Some(LoggerInner {
        log_dir,
        current_date: String::new(),
        file: None,
        retention_days: 7,
    });
}

pub fn set_log_level(level: LogLevel) {
    CURRENT_LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

pub fn get_log_level() -> LogLevel {
    match CURRENT_LOG_LEVEL.load(Ordering::Relaxed) {
        0 => LogLevel::Debug,
        1 => LogLevel::Info,
        2 => LogLevel::Warn,
        3 => LogLevel::Error,
        _ => LogLevel::Off,
    }
}

pub fn should_log(level: LogLevel) -> bool {
    let current = CURRENT_LOG_LEVEL.load(Ordering::Relaxed);
    (level as u8) >= current && current < (LogLevel::Off as u8)
}

pub fn log_message(level: LogLevel, msg: &str) {
    if !should_log(level) {
        return;
    }

    let now = chrono::Local::now();
    let timestamp = now.format("%Y-%m-%d %H:%M:%S%.3f");
    let today = now.format("%Y-%m-%d").to_string();

    let is_foreground = FOREGROUND_LOGGING.load(Ordering::Relaxed);
    if is_foreground {
        if level == LogLevel::Error {
            eprintln!("[{}] [{:5}] {}", timestamp, level.as_str(), msg);
        } else {
            println!("[{}] [{:5}] {}", timestamp, level.as_str(), msg);
        }
    }

    // Write to daily log file
    let mut guard = match LOGGER_STATE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };

    if let Some(inner) = guard.as_mut() {
        // Date rollover check
        if inner.current_date != today || inner.file.is_none() {
            inner.current_date = today.clone();
            let log_filename = format!("wallpaper-shuffler_{}.log", today);
            let log_path = inner.log_dir.join(log_filename);
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .ok();
            inner.file = file;

            // Prune old logs on rollover
            prune_old_logs(&inner.log_dir, inner.retention_days);
        }

        if let Some(file) = inner.file.as_mut() {
            let line = format!("[{}] [{:5}] {}\r\n", timestamp, level.as_str(), msg);
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
}

/// Deletes log files in `log_dir` matching `wallpaper-shuffler_*.log` older than `retention_days`.
pub fn prune_old_logs(log_dir: &Path, retention_days: u64) {
    let entries = match fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let now = SystemTime::now();
    let max_age = Duration::from_secs(retention_days * 24 * 3600);
    let today = chrono::Local::now().date_naive();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };

        if !file_name.starts_with("wallpaper-shuffler_") || !file_name.ends_with(".log") {
            continue;
        }

        // Check age: first check date parsed from filename, fallback to filesystem modified time
        let date_part = file_name
            .strip_prefix("wallpaper-shuffler_")
            .and_then(|s| s.strip_suffix(".log"));

        let is_old = if let Some(part) = date_part {
            if let Ok(file_date) = chrono::NaiveDate::parse_from_str(part, "%Y-%m-%d") {
                let diff_days = (today - file_date).num_days();
                diff_days >= retention_days as i64
            } else {
                check_modified_age(&entry, now, max_age)
            }
        } else {
            check_modified_age(&entry, now, max_age)
        };

        if is_old {
            let _ = fs::remove_file(&path);
        }
    }
}

fn check_modified_age(entry: &fs::DirEntry, now: SystemTime, max_age: Duration) -> bool {
    entry
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age > max_age)
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Debug, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Info, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Warn, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::logger::log_message($crate::logger::LogLevel::Error, &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_parsing() {
        assert_eq!(LogLevel::parse("debug"), LogLevel::Debug);
        assert_eq!(LogLevel::parse("DEBUG"), LogLevel::Debug);
        assert_eq!(LogLevel::parse("info"), LogLevel::Info);
        assert_eq!(LogLevel::parse("warn"), LogLevel::Warn);
        assert_eq!(LogLevel::parse("error"), LogLevel::Error);
        assert_eq!(LogLevel::parse("off"), LogLevel::Off);
        assert_eq!(LogLevel::parse("unknown"), LogLevel::Info);
    }

    #[test]
    fn test_log_file_writing_and_pruning() {
        let temp_dir = std::env::temp_dir().join("wallpaper_shuffler_log_test");
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);

        let test_log = temp_dir.join("wallpaper-shuffler_2020-01-01.log");
        fs::write(&test_log, "old log content").unwrap();

        // Pruning with retention of 1 day should delete 2020 file
        prune_old_logs(&temp_dir, 1);
        assert!(!test_log.exists(), "Old log file should have been pruned");

        let current_log = temp_dir.join("wallpaper-shuffler_2099-01-01.log");
        fs::write(&current_log, "new log content").unwrap();
        prune_old_logs(&temp_dir, 7);
        assert!(current_log.exists(), "Recent log file should remain");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_background_log_writing() {
        let temp_dir = std::env::temp_dir().join("wallpaper_shuffler_log_write_test");
        let _ = fs::remove_dir_all(&temp_dir);
        let _ = fs::create_dir_all(&temp_dir);

        {
            let mut guard = LOGGER_STATE.lock().unwrap();
            *guard = Some(LoggerInner {
                log_dir: temp_dir.clone(),
                current_date: String::new(),
                file: None,
                retention_days: 7,
            });
        }
        FOREGROUND_LOGGING.store(false, Ordering::SeqCst);
        CURRENT_LOG_LEVEL.store(LogLevel::Info as u8, Ordering::SeqCst);

        log_message(LogLevel::Debug, "This debug message should NOT be logged");
        log_message(LogLevel::Info, "This info message SHOULD be logged");

        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let expected_file = temp_dir.join(format!("wallpaper-shuffler_{}.log", today));
        assert!(expected_file.exists(), "Log file should have been created");

        let content = fs::read_to_string(&expected_file).unwrap();
        assert!(!content.contains("This debug message"));
        assert!(content.contains("This info message SHOULD be logged"));
        assert!(content.contains("[INFO ]"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
