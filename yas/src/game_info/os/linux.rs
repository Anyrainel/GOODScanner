use anyhow::{anyhow, Result};
use std::io::stdin;

use crate::game_info::{is_16x9, GameInfo, Platform, UI};
use crate::positioning::Rect;
use crate::utils::{self, X11WindowInfo};

fn is_window_cloud(title: &str) -> bool {
    title.starts_with("云")
}

/// Find the game window by exact title, mirroring the Windows flow in
/// `os/winodws.rs`: log every match, prefer unique matches, fall back to
/// interactive selection when ambiguous.
fn get_window(window_names: &[&str]) -> Result<(u32, bool)> {
    let windows = utils::enumerate_windows()?;

    let candidates: Vec<&X11WindowInfo> = windows
        .iter()
        .filter(|w| window_names.iter().any(|name| w.title == *name))
        .collect();

    if candidates.is_empty() {
        let wayland = std::env::var("WAYLAND_DISPLAY").map(|v| !v.is_empty()).unwrap_or(false);
        let extra = if wayland {
            "\n\
             检测到 Wayland 会话：若游戏以 wine-wayland（纯 Wayland）驱动运行，其窗口对 X11 不可见。\
             请移除 PROTON_ENABLE_WAYLAND=1 或注册表 Graphics=wayland 设置，让游戏回到默认的 winex11/XWayland 路径。\n\
             / Wayland session detected: if the game is running with the wine-wayland (native \
             Wayland) driver its window is invisible to X11. Remove PROTON_ENABLE_WAYLAND=1 or the \
             Graphics=wayland registry setting so the game uses the default winex11/XWayland path."
        } else {
            ""
        };
        return Err(anyhow!(
            "未找到游戏窗口，请确认原神已启动且未最小化。{}\n\
             / Game window not found. Please make sure Genshin Impact is running and not minimized.{}",
            extra, extra,
        ));
    }

    for w in &candidates {
        log_debug!(
            "匹配到窗口: title={:?}, class={:?}, id={:#x}",
            "Matched window: title={:?}, class={:?}, id={:#x}",
            w.title, w.class, w.window,
        );
    }

    if candidates.len() == 1 {
        let w = candidates[0];
        return Ok((w.window, is_window_cloud(&w.title)));
    }

    // Ambiguous — interactive selection (CLI) or first match (GUI).
    println!(
        "{}",
        crate::lang::localize(
            "找到多个符合名称的窗口，请手动选择窗口 / Multiple matching windows found, please select one:"
        )
    );
    for (i, w) in candidates.iter().enumerate() {
        println!("{}: {} [{}]", i, w.title, w.class);
    }
    let mut index = String::new();
    let idx = match stdin().read_line(&mut index) {
        Ok(_) => index.trim().parse::<usize>().unwrap_or(0),
        Err(_) => 0,
    };
    let idx = idx.min(candidates.len() - 1);
    let w = candidates[idx];
    Ok((w.window, is_window_cloud(&w.title)))
}

pub fn get_game_info(window_names: &[&str]) -> Result<GameInfo> {
    let (window_id, is_cloud) = get_window(window_names)?;

    let rect = utils::get_client_rect(window_id)?;

    if !is_16x9(rect.to_rect_usize().size()) {
        log_error!(
            "游戏窗口内部区域为 {}x{}，不是16:9比例。本工具仅支持16:9分辨率（如1920×1080、2560×1440、3840×2160）。\n\
             请在游戏设置中切换到16:9分辨率后重试。",
            "Game window client area is {}x{}, which is not 16:9. This tool only supports 16:9 aspect ratio \
             (e.g. 1920×1080, 2560×1440, 3840×2160).\n\
             Please switch to a 16:9 resolution in game settings and try again.",
            rect.width, rect.height,
        );
        return Err(anyhow!(
            "不支持的分辨率: {}x{}（内部区域）。请使用16:9分辨率（如1920×1080、2560×1440、3840×2160）。\n\
             / Unsupported resolution: {}x{} (client area). Only 16:9 is supported (e.g. 1920×1080, 2560×1440, 3840×2160).",
            rect.width, rect.height, rect.width, rect.height
        ));
    }

    Ok(GameInfo {
        window: Rect::new(rect.left, rect.top, rect.width, rect.height),
        is_cloud,
        ui: UI::Desktop,
        platform: Platform::Linux,
        window_id,
    })
}
