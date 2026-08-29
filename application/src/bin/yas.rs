// Hide console window in GUI mode. CLI mode reattaches below.
#![windows_subsystem = "windows"]

use genshin_scanner::cli::GoodScannerApplication;
use yas::utils::press_any_key_to_continue;

/// Attach to the parent process's console (e.g. cmd.exe, PowerShell).
/// If no parent console exists, allocate a new one.
/// This is needed because `windows_subsystem = "windows"` detaches from the console.
#[cfg(windows)]
fn attach_console() {
    use std::os::raw::c_int;
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> c_int;
        fn AllocConsole() -> c_int;
    }
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            AllocConsole();
        }
    }
}

fn init_cli() {
    #[cfg(windows)]
    attach_console();

    // Set global language from config before logger init
    let config = genshin_scanner::cli::load_config_or_default();
    yas::lang::set_lang(&config.lang);

    let level = if config.verbose {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    };
    let logger_result = env_logger::Builder::new()
        .filter_level(level)
        .format(|buf, record| {
            use std::io::Write;
            let raw = format!("{}", record.args());
            let msg = yas::lang::localize_log_message(record.target(), &raw);
            writeln!(buf, "{}", msg)
        })
        .try_init();
    if let Err(error) = logger_result {
        eprintln!(
            "{}\n\n{}\n{}",
            yas::lang::localize(
                "命令行错误记录功能无法初始化；程序将继续运行。 / Command-line error logging could not be initialized; the application will continue."
            ),
            yas::lang::localize("完整错误详情: / Full error details:"),
            error
        );
    }

    // Install a custom panic hook so that panics (from unwrap, expect, panic!, etc.)
    // print the error and wait for user input before the process exits.
    // Without this, the console window closes immediately and users can't see the error.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        good_tools_app::gui::state::record_panic_details(info);
        eprintln!(
            "{}\n\n{}",
            yas::lang::localize(
                "程序因意外的内部错误而停止。请复制完整错误以搜索或寻求帮助。 / The application stopped because of an unexpected internal error. Copy the full error to search or ask for help."
            ),
            yas::lang::localize("完整错误详情: / Full error details:")
        );
        default_hook(info);
    }));
}

pub fn main() {
    // No CLI args → launch GUI; any args → CLI mode
    if std::env::args().len() == 1 {
        good_tools_app::gui::run_gui();
        return;
    }

    // CLI mode: attach console and run
    init_cli();
    let command = GoodScannerApplication::build_command();
    let matches = match command.try_get_matches() {
        Ok(m) => m,
        Err(e) => {
            let is_error = e.use_stderr();
            if is_error {
                eprintln!(
                    "{}\n\n{}\n{}",
                    yas::lang::localize(
                        "命令行参数无法识别。请检查下面的用法说明后重试。 / The command-line arguments could not be understood. Check the usage details below, then retry."
                    ),
                    yas::lang::localize("完整错误详情: / Full error details:"),
                    e
                );
            } else {
                println!("{}", e);
            }
            press_any_key_to_continue();
            std::process::exit(if is_error { 1 } else { 0 });
        },
    };

    let application = GoodScannerApplication::new(matches);
    let result = good_tools_app::gui::worker::run_cli_with_safety_net(
        good_tools_app::gui::state::UiText::new(
            "扫描器未能完成请求。请复制完整错误以搜索或寻求帮助。",
            "The scanner could not complete the request. Copy the full error to search or ask for help.",
        ),
        move || application.run(),
    );
    match result {
        Ok(_) => {
            press_any_key_to_continue();
        },
        Err(error) => {
            let lang = if yas::lang::is_en() {
                good_tools_app::gui::state::Lang::En
            } else {
                good_tools_app::gui::state::Lang::Zh
            };
            let full_error = error.copy_text(lang);
            log::error!(target: yas::lang::LOCALIZED_LOG_TARGET, "{}", full_error);
            if error.is_native_exception() {
                match good_tools_app::gui::state::persist_error_report(
                    "cli_native_error.txt",
                    &full_error,
                ) {
                    Ok(path) => {
                        eprintln!(
                            "{}\n{}",
                            yas::lang::localize(
                                "完整错误也已保存到以下文件: / The full error was also saved here:"
                            ),
                            path.display()
                        );
                    },
                    Err(report_error) => {
                        eprintln!(
                            "{}\n{:#}",
                            yas::lang::localize(
                                "程序还无法保存错误报告。请复制控制台中的完整错误。完整错误详情: / The application also could not save the error report. Copy the complete error from this console. Full error details:"
                            ),
                            report_error,
                        );
                    },
                }
                // A nested native worker may have exited without releasing a
                // channel or lock. Exit promptly rather than letting the game
                // controller continue while waiting for console input.
                std::process::exit(1);
            }
            press_any_key_to_continue();
        },
    }
}
