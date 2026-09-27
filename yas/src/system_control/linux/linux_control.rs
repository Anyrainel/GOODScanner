//! Input injection for Linux with two backends:
//!
//! - **XTEST** (X11 sessions): fake input through the X server. Zero external
//!   dependencies, exact absolute positioning.
//! - **ydotool** (Wayland sessions): compositors such as KWin/Mutter ignore
//!   XTEST fake input, so injection goes through the ydotoold uinput daemon.
//!
//! The backend is picked on first use: an X11 session prefers XTEST (with a
//! live effectiveness check), a Wayland session prefers ydotool and falls
//! back to XTEST only if the compositor happens to honor it. The public
//! method surface mirrors `WindowsSystemControl`; `enigo::Key` stays the key
//! type so callers remain platform-independent.

use enigo::Key;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
    MOTION_NOTIFY_EVENT,
};

use crate::system_control::linux::ydotool::{
    char_to_evdev_code, YdotoolClient, BTN_LEFT, KEY_BACKSPACE, KEY_CAPSLOCK, KEY_DELETE,
    KEY_DOWN, KEY_END, KEY_ENTER, KEY_ESC, KEY_F1, KEY_HOME, KEY_LEFT, KEY_LEFTALT, KEY_LEFTCTRL,
    KEY_LEFTMETA, KEY_LEFTSHIFT, KEY_PAGEDOWN, KEY_PAGEUP, KEY_RIGHT, KEY_SPACE, KEY_TAB, KEY_UP,
};
use crate::utils;

// X buttons: 1=left, 3=right; wheel is 4 (up) / 5 (down).
const BUTTON_LEFT: u8 = 1;
const BUTTON_WHEEL_UP: u8 = 4;
const BUTTON_WHEEL_DOWN: u8 = 5;

enum InputBackend {
    XTest,
    Ydotool(YdotoolClient),
}

impl InputBackend {
    /// Pick a working backend: X11 session → XTEST; Wayland session →
    /// ydotool first (most compositors ignore XTEST), XTEST as fallback.
    fn select() -> anyhow::Result<Self> {
        let wayland_session = std::env::var("WAYLAND_DISPLAY")
            .map(|v| !v.is_empty())
            .unwrap_or(false);

        if wayland_session {
            match YdotoolClient::connect() {
                Ok(client) => {
                    log_info!(
                        "输入后端: ydotool（Wayland 会话）",
                        "input backend: ydotool (Wayland session)"
                    );
                    return Ok(Self::Ydotool(client));
                },
                Err(e) => {
                    // XTEST is almost certainly dead here, but some compositors
                    // do forward it — check before giving up.
                    if utils::xtest_available() && utils::xtest_effective() {
                        log_info!(
                            "输入后端: XTEST（当前合成器支持）",
                            "input backend: XTEST (this compositor honors it)"
                        );
                        return Ok(Self::XTest);
                    }
                    return Err(e.context(
                        "Wayland 会话无法模拟输入。/ Cannot inject input in a Wayland session.",
                    ));
                },
            }
        }

        if utils::xtest_available() && utils::xtest_effective() {
            log_info!(
                "输入后端: XTEST（X11 会话）",
                "input backend: XTEST (X11 session)"
            );
            return Ok(Self::XTest);
        }

        // Odd X11 setups (nested servers, some compositors) — try ydotool.
        match YdotoolClient::connect() {
            Ok(client) => {
                log_info!(
                    "输入后端: ydotool（XTEST 无效，回退）",
                    "input backend: ydotool (XTEST ineffective, fallback)"
                );
                Ok(Self::Ydotool(client))
            },
            Err(e) => Err(anyhow::Error::msg(format!(
                "XTEST 注入无效且未安装 ydotool；请使用 X11 会话，或安装并启用 ydotool 服务。\
                 （{e}）\n\
                 / XTEST injection is ineffective and ydotool is not available; use an X11 \
                 session, or install and enable the ydotool service. ({e})"
            ))),
        }
    }
}

// X keysyms (see X11/keysymdef.h).
const XK_BACKSPACE: u32 = 0xff08;
const XK_TAB: u32 = 0xff09;
const XK_RETURN: u32 = 0xff0d;
const XK_ESCAPE: u32 = 0xff1b;
const XK_HOME: u32 = 0xff50;
const XK_LEFT: u32 = 0xff51;
const XK_UP: u32 = 0xff52;
const XK_RIGHT: u32 = 0xff53;
const XK_DOWN: u32 = 0xff54;
const XK_PAGE_UP: u32 = 0xff55;
const XK_PAGE_DOWN: u32 = 0xff56;
const XK_END: u32 = 0xff57;
const XK_DELETE: u32 = 0xffff;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_CONTROL_L: u32 = 0xffe3;
const XK_ALT_L: u32 = 0xffe9;
const XK_SUPER_L: u32 = 0xffeb;
const XK_CAPS_LOCK: u32 = 0xffe5;
const XK_F1: u32 = 0xffbe;

/// Resolve a key to (X keysym, evdev code) — both backends are served from
/// one table.
fn key_to_codes(key: &Key) -> Result<Option<(u32, u16)>, String> {
    let codes = match key {
        Key::Layout(c) => {
            // Latin-1 keysyms map 1:1; CJK etc. would need an IME.
            let code = *c as u32;
            if (0x20..=0xff).contains(&code) {
                let evdev = char_to_evdev_code(*c)
                    .ok_or_else(|| format!("cannot type character {c:?} (no evdev keycode)"))?;
                Some((code, evdev))
            } else {
                return Err(format!("cannot type character {c:?} (non-latin1 keysym)"));
            }
        },
        Key::Return => Some((XK_RETURN, KEY_ENTER)),
        Key::Escape => Some((XK_ESCAPE, KEY_ESC)),
        Key::Tab => Some((XK_TAB, KEY_TAB)),
        Key::Space => Some((0x20, KEY_SPACE)),
        Key::Backspace => Some((XK_BACKSPACE, KEY_BACKSPACE)),
        Key::Delete => Some((XK_DELETE, KEY_DELETE)),
        Key::Home => Some((XK_HOME, KEY_HOME)),
        Key::End => Some((XK_END, KEY_END)),
        Key::PageUp => Some((XK_PAGE_UP, KEY_PAGEUP)),
        Key::PageDown => Some((XK_PAGE_DOWN, KEY_PAGEDOWN)),
        Key::LeftArrow => Some((XK_LEFT, KEY_LEFT)),
        Key::UpArrow => Some((XK_UP, KEY_UP)),
        Key::RightArrow => Some((XK_RIGHT, KEY_RIGHT)),
        Key::DownArrow => Some((XK_DOWN, KEY_DOWN)),
        Key::Shift => Some((XK_SHIFT_L, KEY_LEFTSHIFT)),
        Key::Control => Some((XK_CONTROL_L, KEY_LEFTCTRL)),
        Key::Alt => Some((XK_ALT_L, KEY_LEFTALT)),
        Key::Option => Some((XK_ALT_L, KEY_LEFTALT)),
        Key::Meta => Some((XK_SUPER_L, KEY_LEFTMETA)),
        Key::CapsLock => Some((XK_CAPS_LOCK, KEY_CAPSLOCK)),
        Key::F1 => Some((XK_F1, KEY_F1)),
        Key::F2 => Some((XK_F1 + 1, KEY_F1 + 1)),
        Key::F3 => Some((XK_F1 + 2, KEY_F1 + 2)),
        Key::F4 => Some((XK_F1 + 3, KEY_F1 + 3)),
        Key::F5 => Some((XK_F1 + 4, KEY_F1 + 4)),
        Key::F6 => Some((XK_F1 + 5, KEY_F1 + 5)),
        Key::F7 => Some((XK_F1 + 6, KEY_F1 + 6)),
        Key::F8 => Some((XK_F1 + 7, KEY_F1 + 7)),
        Key::F9 => Some((XK_F1 + 8, KEY_F1 + 8)),
        Key::F10 => Some((XK_F1 + 9, KEY_F1 + 9)),
        Key::F11 => Some((XK_F1 + 10, KEY_F1 + 10)),
        Key::F12 => Some((XK_F1 + 11, KEY_F1 + 11)),
        _ => return Err(format!("unsupported key {key:?} on Linux")),
    };
    Ok(codes)
}

pub struct LinuxControl {
    /// Lazily selected on first injected event; errors are surfaced there
    /// (the constructor mirrors the infallible Windows API).
    backend: Option<InputBackend>,
    backend_error: Option<String>,
}

impl LinuxControl {
    pub fn new() -> LinuxControl {
        LinuxControl {
            backend: None,
            backend_error: None,
        }
    }

    fn with_backend<T>(
        &mut self,
        f: impl FnOnce(&mut InputBackend) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        if self.backend.is_none() {
            match InputBackend::select() {
                Ok(b) => self.backend = Some(b),
                Err(e) => {
                    if self.backend_error.is_none() {
                        self.backend_error = Some(format!("{e:#}"));
                    }
                    return Err(anyhow::Error::msg(
                        self.backend_error.clone().expect("just set"),
                    ));
                },
            }
        }
        f(self.backend.as_mut().expect("backend was just initialized"))
    }

    pub fn mouse_move_to(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        self.with_backend(|backend| match backend {
            InputBackend::XTest => utils::fake_input(MOTION_NOTIFY_EVENT, 0, x as i16, y as i16),
            InputBackend::Ydotool(client) => client.mouse_move_abs(x, y),
        })
    }

    pub fn mouse_click(&mut self) -> anyhow::Result<()> {
        self.with_backend(|backend| match backend {
            InputBackend::XTest => {
                // Hold briefly, mirroring WindowsSystemControl, so the game
                // registers the press.
                utils::fake_input(BUTTON_PRESS_EVENT, BUTTON_LEFT, 0, 0)?;
                utils::sleep(20);
                utils::fake_input(BUTTON_RELEASE_EVENT, BUTTON_LEFT, 0, 0)
            },
            InputBackend::Ydotool(client) => {
                client.button(BTN_LEFT, true)?;
                utils::sleep(20);
                client.button(BTN_LEFT, false)
            },
        })
    }

    pub fn mouse_scroll(&mut self, amount: i32, _try_find: bool) -> anyhow::Result<()> {
        self.with_backend(|backend| match backend {
            InputBackend::XTest => {
                let button = if amount >= 0 { BUTTON_WHEEL_UP } else { BUTTON_WHEEL_DOWN };
                for _ in 0..amount.abs() {
                    utils::fake_input(BUTTON_PRESS_EVENT, button, 0, 0)?;
                    utils::fake_input(BUTTON_RELEASE_EVENT, button, 0, 0)?;
                }
                anyhow::Ok(())
            },
            InputBackend::Ydotool(client) => client.wheel(amount),
        })
    }

    pub fn mouse_scroll_wheel_delta(&mut self, delta: i32) -> anyhow::Result<()> {
        let notches = if delta.abs() >= 120 {
            delta / 120
        } else if delta == 0 {
            0
        } else {
            delta.signum()
        };
        if notches != 0 {
            self.mouse_scroll(notches, false)?;
        }
        anyhow::Ok(())
    }

    pub fn key_press(&mut self, key: Key) -> anyhow::Result<()> {
        let codes = key_to_codes(&key).map_err(anyhow::Error::msg)?;
        let Some((keysym, evdev)) = codes else {
            return anyhow::Ok(());
        };

        self.with_backend(|backend| match backend {
            InputBackend::XTest => {
                let (keycode, needs_shift) = utils::keysym_to_keycode(keysym)?;
                let shift = if needs_shift {
                    utils::keysym_to_keycode(XK_SHIFT_L).ok().map(|(code, _)| code)
                } else {
                    None
                };

                if let Some(shift_code) = shift {
                    utils::fake_input(KEY_PRESS_EVENT, shift_code, 0, 0)?;
                }
                utils::fake_input(KEY_PRESS_EVENT, keycode, 0, 0)?;
                utils::sleep(10);
                utils::fake_input(KEY_RELEASE_EVENT, keycode, 0, 0)?;
                if let Some(shift_code) = shift {
                    utils::fake_input(KEY_RELEASE_EVENT, shift_code, 0, 0)?;
                }
                anyhow::Ok(())
            },
            InputBackend::Ydotool(client) => {
                client.key(evdev, true)?;
                utils::sleep(10);
                client.key(evdev, false)
            },
        })
    }
}
