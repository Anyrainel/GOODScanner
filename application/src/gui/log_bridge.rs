use std::cell::Cell;
use std::sync::Arc;

use log::{Level, LevelFilter, Log, Metadata, Record};

use super::state::{LogEntry, LogSource, LogStore};

thread_local! {
    /// Per-thread override for log routing. When set, logs on this thread
    /// (regardless of module path) are routed to the specified source.
    /// Used by `worker::spawn_server` so that logs emitted from `genshin_scanner::cli`
    /// during server startup are classified as Manager.
    static LOG_SOURCE_OVERRIDE: Cell<Option<LogSource>> = const { Cell::new(None) };
}

/// Set the log source override for the current thread.
pub fn set_thread_log_source(src: LogSource) {
    LOG_SOURCE_OVERRIDE.with(|c| c.set(Some(src)));
}

/// Update the global log level filter (call when verbose checkbox toggles).
pub fn set_verbose(verbose: bool) {
    let level = if verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    log::set_max_level(level);
}

fn classify(record: &Record) -> LogSource {
    if let Some(src) = LOG_SOURCE_OVERRIDE.with(|c| c.get()) {
        return src;
    }
    match record.module_path() {
        Some(p)
            if p.starts_with("genshin_scanner::manager")
                || p.starts_with("genshin_scanner::server") =>
        {
            LogSource::Manager
        },
        _ => LogSource::Scanner,
    }
}

/// Custom logger that routes `log` crate output to per-tab buffers for GUI display,
/// and optionally to a file in the `log/` directory.
pub struct GuiLogger {
    scanner: Arc<LogStore>,
    manager: Arc<LogStore>,
    file_sink: Option<FileLogSink>,
}

struct FileLogSink {
    queue: Arc<crossbeam_queue::SegQueue<String>>,
    active: Arc<std::sync::atomic::AtomicBool>,
}

impl GuiLogger {
    pub fn new(scanner: Arc<LogStore>, manager: Arc<LogStore>, _max_lines: usize) -> Self {
        // Create log/ directory and open a timestamped log file
        let file_sink = match std::fs::create_dir_all("log") {
            Err(error) => {
                push_file_logging_error(
                    &scanner,
                    &manager,
                    "无法创建日志文件夹。错误仍会显示在程序中，但不会保存到磁盘。",
                    "The log folder could not be created. Errors will remain visible in the application but will not be saved to disk.",
                    &error,
                );
                None
            },
            Ok(()) => {
                let ts = format_timestamp().replace(':', "-");
                let path = format!("log/run_{}.log", ts);
                match std::fs::File::create(&path) {
                    Err(error) => {
                        push_file_logging_error(
                            &scanner,
                            &manager,
                            "无法创建本次运行的日志文件。错误仍会显示在程序中，但不会保存到磁盘。",
                            "The log file for this run could not be created. Errors will remain visible in the application but will not be saved to disk.",
                            &error,
                        );
                        None
                    },
                    Ok(mut file) => {
                        let queue = Arc::new(crossbeam_queue::SegQueue::<String>::new());
                        let writer_queue = queue.clone();
                        let active = Arc::new(std::sync::atomic::AtomicBool::new(true));
                        let writer_active = active.clone();
                        let writer_scanner = scanner.clone();
                        let writer_manager = manager.clone();
                        match std::thread::Builder::new()
                            .name("gui-log-writer".to_owned())
                            .spawn(move || loop {
                                let mut wrote = false;
                                while let Some(line) = writer_queue.pop() {
                                    use std::io::Write;
                                    if let Err(error) = writeln!(file, "{}", line) {
                                        push_file_logging_error(
                                            &writer_scanner,
                                            &writer_manager,
                                            "日志文件无法继续写入。完整错误仍保留在程序内的日志中。",
                                            "The log file could not be written. Complete errors remain available in the in-app log.",
                                            &error,
                                        );
                                        writer_active.store(
                                            false,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                        return;
                                    }
                                    wrote = true;
                                }
                                if wrote {
                                    use std::io::Write;
                                    if let Err(error) = file.flush() {
                                        push_file_logging_error(
                                            &writer_scanner,
                                            &writer_manager,
                                            "日志文件无法保存到磁盘。完整错误仍保留在程序内的日志中。",
                                            "The log file could not be flushed to disk. Complete errors remain available in the in-app log.",
                                            &error,
                                        );
                                        writer_active.store(
                                            false,
                                            std::sync::atomic::Ordering::Release,
                                        );
                                        return;
                                    }
                                }
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }) {
                            Ok(_handle) => Some(FileLogSink { queue, active }),
                            Err(error) => {
                                push_file_logging_error(
                                    &scanner,
                                    &manager,
                                    "日志写入后台任务无法启动。错误仍会显示在程序中，但不会保存到磁盘。",
                                    "The background log writer could not start. Errors will remain visible in the application but will not be saved to disk.",
                                    &error,
                                );
                                None
                            },
                        }
                    },
                }
            },
        };
        Self {
            scanner,
            manager,
            file_sink,
        }
    }

    pub fn init(self, verbose: bool) -> Result<(), log::SetLoggerError> {
        log::set_boxed_logger(Box::new(self))?;
        let level = if verbose {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        };
        log::set_max_level(level);
        Ok(())
    }
}

fn push_file_logging_error(
    scanner: &LogStore,
    manager: &LogStore,
    hint_zh: &str,
    hint_en: &str,
    error: &dyn std::fmt::Display,
) {
    let message = if yas::lang::is_en() {
        format!("{hint_en}\n\nFull error details:\n{error}")
    } else {
        format!("{hint_zh}\n\n完整错误详情:\n{error}")
    };
    let timestamp = format_timestamp();
    scanner.push(LogEntry {
        level: Level::Error,
        message: message.clone(),
        timestamp: timestamp.clone(),
        source: LogSource::Scanner,
    });
    manager.push(LogEntry {
        level: Level::Error,
        message,
        timestamp,
        source: LogSource::Manager,
    });
}

impl Log for GuiLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        let max = log::max_level();
        if metadata.level() > max {
            return false;
        }
        // Our crates pass through at Info+ (and Debug when verbose).
        // Third-party crates: only Warn and Error.
        match metadata.level() {
            Level::Error | Level::Warn => true,
            Level::Info | Level::Debug => {
                matches!(metadata.target(),
                    t if t.starts_with("yas")
                        || t.starts_with("genshin_scanner")
                        || t.starts_with("good_tools_app")
                        || t.starts_with("yas_core"))
            },
            Level::Trace => false,
        }
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let raw = format!("{}", record.args());
            let localized = yas::lang::localize_log_message(record.target(), &raw);
            let ts = format_timestamp();
            let source = classify(record);
            let entry = LogEntry {
                level: record.level(),
                message: localized.clone(),
                timestamp: ts.clone(),
                source,
            };
            let buf = match source {
                LogSource::Scanner => &self.scanner,
                LogSource::Manager => &self.manager,
            };
            buf.push(entry);
            // File I/O is owned by a dedicated thread; producers only push to
            // a lock-free queue, so neither contention nor a native worker
            // exit can discard the sole copy of an error.
            if let Some(sink) = &self.file_sink {
                if sink.active.load(std::sync::atomic::Ordering::Acquire) {
                    sink.queue
                        .push(format!("{} [{}] {}", ts, record.level(), localized));
                }
            }
        }
    }

    fn flush(&self) {
        // The dedicated writer flushes after every drained batch.
    }
}

#[cfg(windows)]
fn format_timestamp() -> String {
    use std::mem::MaybeUninit;
    #[repr(C)]
    struct SystemTime {
        w_year: u16,
        w_month: u16,
        w_day_of_week: u16,
        w_day: u16,
        w_hour: u16,
        w_minute: u16,
        w_second: u16,
        w_milliseconds: u16,
    }
    extern "system" {
        fn GetLocalTime(lp_system_time: *mut SystemTime);
    }
    let mut st = MaybeUninit::<SystemTime>::uninit();
    unsafe {
        GetLocalTime(st.as_mut_ptr());
        let st = st.assume_init();
        format!("{:02}:{:02}:{:02}", st.w_hour, st.w_minute, st.w_second)
    }
}

#[cfg(not(windows))]
fn format_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs_of_day = now % 86400;
    let hours = secs_of_day / 3600;
    let minutes = (secs_of_day % 3600) / 60;
    let seconds = secs_of_day % 60;
    format!("{:02}:{:02}:{:02}", hours, minutes, seconds)
}
