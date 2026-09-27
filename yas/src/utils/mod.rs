pub use misc::*;
use serde::Deserialize;
use std::fmt::Arguments;
use std::fs;
use std::io::stdin;
use std::path::Path;
use std::process;
use std::thread;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(target_os = "linux")]
mod linux_x11;
#[cfg(target_os = "linux")]
pub use linux_x11::*;

#[cfg(target_os = "linux")]
pub fn available_memory_bytes() -> Option<u64> {
    // /proc/meminfo MemAvailable (kB).
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemAvailable:") {
            let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn available_memory_bytes() -> Option<u64> {
    None
}

mod misc;

pub fn sleep(ms: u32) {
    thread::sleep(Duration::from_millis(ms as u64));
}

pub fn read_file_to_string<P: AsRef<Path>>(path: P) -> String {
    fs::read_to_string(path).unwrap()
}

pub fn quit() -> ! {
    let mut s: String = String::new();
    stdin().read_line(&mut s).unwrap();
    process::exit(0);
}

#[doc(hidden)]
pub fn error_and_quit_internal(args: Arguments) -> ! {
    panic!("错误 / Error: {}", args);
}

#[macro_export]
macro_rules! error_and_quit {
    ($($arg:tt)*) => (
        $crate::utils::error_and_quit_internal(format_args!($($arg)*))
    );
}

#[derive(Deserialize)]
pub struct GithubTag {
    pub name: String,
}

pub fn ensure_dir(path: &str) {
    if !std::path::Path::new(path).exists() {
        fs::create_dir_all(path).unwrap();
    }
}
