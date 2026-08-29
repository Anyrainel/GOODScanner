use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;

use super::state::{Lang, UiError, UiText, UpdateState};
use super::widgets;

/// Show the update notification banner when an update is available.
///
/// Call this from the `eframe::App::update` method, before the central panel.
/// `update_state` is the shared state that tracks the update lifecycle.
pub fn show(ctx: &egui::Context, l: Lang, update_state: &Arc<Mutex<UpdateState>>) {
    let state_snapshot = update_state.lock().unwrap().clone();

    let show = !matches!(state_snapshot, UpdateState::None | UpdateState::Checking);
    if !show {
        return;
    }

    egui::TopBottomPanel::top("update_banner").show(ctx, |ui| match state_snapshot {
        UpdateState::Available {
            ref latest_version,
            ref download_url,
        } => {
            ui.horizontal(|ui| {
                let current = genshin_scanner::updater::current_version_display();
                ui.label(
                    egui::RichText::new(l.t(
                        &format!("发现新版本: {} → {}", current, latest_version),
                        &format!("Update available: {} → {}", current, latest_version),
                    ))
                    .color(egui::Color32::from_rgb(255, 200, 50)),
                );
                if ui.button(l.t("下载更新", "Download Update")).clicked() {
                    let arc = update_state.clone();
                    let url = download_url.clone();
                    let lang = l;
                    *update_state.lock().unwrap() = UpdateState::Downloading;
                    let worker_state = arc.clone();
                    let spawn_result = std::thread::Builder::new()
                        .name("update-download".to_owned())
                        .spawn(move || {
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            genshin_scanner::updater::download_and_replace(&url)
                        }));
                        match result {
                            Ok(Ok(exe_path)) => {
                                *worker_state.lock().unwrap() = UpdateState::ShowingDialog;
                                show_restart_dialog(exe_path, worker_state, lang);
                            },
                            Ok(Err(error)) => {
                                *worker_state.lock().unwrap() = UpdateState::Failed(
                                    UiError::from_anyhow(
                                        UiText::new(
                                            "更新无法下载或安装。请检查网络连接、磁盘空间和安全软件设置，然后重启程序以重试。",
                                            "The update could not be downloaded or installed. Check the network connection, disk space, and security software, then restart the application to retry.",
                                        ),
                                        &error,
                                    ),
                                );
                            },
                            Err(panic_info) => {
                                *worker_state.lock().unwrap() =
                                    UpdateState::Failed(UiError::from_panic(
                                        UiText::new(
                                            "更新任务因意外的内部错误而停止。请复制完整错误并报告此问题。",
                                            "The update task stopped because of an unexpected internal error. Copy the full error and report this problem.",
                                        ),
                                        panic_info.as_ref(),
                                    ));
                            },
                        }
                    });
                    if let Err(error) = spawn_result {
                        *arc.lock().unwrap() = UpdateState::Failed(UiError::from_error(
                            UiText::new(
                                "更新后台任务无法启动。请检查系统资源，然后重试。",
                                "The update background task could not start. Check available system resources, then retry.",
                            ),
                            error,
                        ));
                    }
                }
                if ui.button(l.t("跳过", "Skip")).clicked() {
                    *update_state.lock().unwrap() = UpdateState::None;
                }
            });
        },
        UpdateState::Downloading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(l.t("正在下载更新...", "Downloading update..."));
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        },
        UpdateState::ShowingDialog => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    egui::RichText::new(l.t("更新已就绪...", "Update ready..."))
                        .color(egui::Color32::from_rgb(100, 255, 100)),
                );
            });
        },
        UpdateState::Ready => {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(l.t(
                        "更新已就绪，请重启程序。",
                        "Update ready. Please restart the application.",
                    ))
                    .color(egui::Color32::from_rgb(100, 255, 100)),
                );
            });
        },
        UpdateState::Failed(ref error) => {
            ui.horizontal(|ui| {
                if ui.button(l.t("关闭", "Dismiss")).clicked() {
                    *update_state.lock().unwrap() = UpdateState::None;
                }
            });
            widgets::error_card(ui, l, error);
        },
        _ => {},
    });
}

/// Spawn a background update check.  Returns immediately.
pub fn spawn_check(asset_name: &'static str, update_state: &Arc<Mutex<UpdateState>>) {
    let state = update_state.clone();
    let spawn_failure_state = state.clone();
    let spawn_result = std::thread::Builder::new()
        .name("update-check".to_owned())
        .spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            genshin_scanner::updater::check_for_update(asset_name)
        }));
        match result {
            Ok(Ok(genshin_scanner::updater::UpdateStatus::UpdateAvailable {
                latest_version,
                download_url,
                ..
            })) => {
                *state.lock().unwrap() = UpdateState::Available {
                    latest_version,
                    download_url,
                };
            },
            Ok(Ok(_)) => {
                *state.lock().unwrap() = UpdateState::None;
            },
            Ok(Err(error)) => {
                yas::log_debug!(
                    "无法检查更新。完整错误详情: {:#}",
                    "Could not check for updates. Full error details: {:#}",
                    error
                );
                *state.lock().unwrap() = UpdateState::Failed(UiError::from_anyhow(
                    UiText::new(
                        "无法检查更新。当前版本仍可继续使用；请检查网络连接，重启程序以重试，或关闭此提示。",
                        "Updates could not be checked. You can keep using this version; check the network connection, restart the application to retry, or dismiss this message.",
                    ),
                    &error,
                ));
            },
            Err(panic_info) => {
                *state.lock().unwrap() = UpdateState::Failed(UiError::from_panic(
                    UiText::new(
                        "检查更新的后台任务因意外的内部错误而停止。当前版本仍可继续使用。",
                        "The background update check stopped because of an unexpected internal error. You can keep using the current version.",
                    ),
                    panic_info.as_ref(),
                ));
            },
        }
    });
    if let Err(error) = spawn_result {
        *spawn_failure_state.lock().unwrap() = UpdateState::Failed(UiError::from_error(
            UiText::new(
                "检查更新的后台任务无法启动。当前版本仍可继续使用；请检查系统资源后重启程序。",
                "The background update check could not start. You can keep using the current version; check system resources, then restart the application.",
            ),
            error,
        ));
    }
}

/// Show a native OS dialog asking the user to restart now or later.
fn show_restart_dialog(exe_path: PathBuf, update_state: Arc<Mutex<UpdateState>>, lang: Lang) {
    let (title, description) = match lang {
        Lang::Zh => ("更新完成", "更新已下载完成。是否立即重启？"),
        Lang::En => (
            "Update Complete",
            "The update has been downloaded. Restart now?",
        ),
    };

    let result = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Info)
        .set_title(title)
        .set_description(description)
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();

    match result {
        rfd::MessageDialogResult::Yes => {
            yas::log_info!("用户选择立即重启", "User chose to restart now");
            match std::process::Command::new(&exe_path).spawn() {
                Ok(_) => std::process::exit(0),
                Err(e) => {
                    yas::log_error!("启动新版本失败: {}", "Failed to launch new version: {}", e);
                    *update_state.lock().unwrap() = UpdateState::Failed(
                        UiError::from_error(
                            UiText::new(
                                "更新已安装，但新版本无法自动启动。请手动重新打开程序。",
                                "The update was installed, but the new version could not start automatically. Reopen the application manually.",
                            ),
                            e,
                        ),
                    );
                },
            }
        },
        _ => {
            yas::log_info!("用户选择稍后重启", "User chose to restart later");
            *update_state.lock().unwrap() = UpdateState::Ready;
        },
    }
}
