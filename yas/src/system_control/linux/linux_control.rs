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
//!
//! Key resolution: the XTEST path always asks the X server's own keymap
//! (`keysym_to_keycode`), so it follows the session layout. The ydotool path
//! prefers the same keymap via XWayland (X keycodes are evdev keycodes + 8)
//! and falls back to a US-QWERTY table when X is unreachable.

use enigo::Key;
use evdev::KeyCode;
use x11rb::protocol::xproto::{
    BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
    MOTION_NOTIFY_EVENT,
};
use xkeysym::{key as xk, Keysym};

use crate::system_control::linux::ydotool::{qwerty_evdev_code, YdotoolClient};
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

/// Resolve a key to `(keysym, evdev code)`; `ch` carries the Layout character
/// for layout-aware resolution. Named keys map to fixed evdev codes.
fn key_to_codes(key: &Key) -> Result<Option<(Keysym, Option<KeyCode>, Option<char>)>, String> {
    // (keysym, fixed evdev code for named keys, layout char)
    Ok(Some(match key {
        Key::Layout(c) => {
            // xkb-compatible keysym derivation; chars without a keysym
            // (and no IME) cannot be typed.
            match Keysym::from_char(*c) {
                keysym if keysym.raw() != xk::VoidSymbol => (keysym, None, Some(*c)),
                _ => return Err(format!("cannot type character {c:?} (no keysym)")),
            }
        },
        Key::Return => (Keysym::Return, Some(KeyCode::KEY_ENTER), None),
        Key::Escape => (Keysym::Escape, Some(KeyCode::KEY_ESC), None),
        Key::Tab => (Keysym::Tab, Some(KeyCode::KEY_TAB), None),
        Key::Space => (Keysym::space, Some(KeyCode::KEY_SPACE), None),
        Key::Backspace => (Keysym::BackSpace, Some(KeyCode::KEY_BACKSPACE), None),
        Key::Delete => (Keysym::Delete, Some(KeyCode::KEY_DELETE), None),
        Key::Home => (Keysym::Home, Some(KeyCode::KEY_HOME), None),
        Key::End => (Keysym::End, Some(KeyCode::KEY_END), None),
        Key::PageUp => (Keysym::Page_Up, Some(KeyCode::KEY_PAGEUP), None),
        Key::PageDown => (Keysym::Page_Down, Some(KeyCode::KEY_PAGEDOWN), None),
        Key::LeftArrow => (Keysym::Left, Some(KeyCode::KEY_LEFT), None),
        Key::UpArrow => (Keysym::Up, Some(KeyCode::KEY_UP), None),
        Key::RightArrow => (Keysym::Right, Some(KeyCode::KEY_RIGHT), None),
        Key::DownArrow => (Keysym::Down, Some(KeyCode::KEY_DOWN), None),
        Key::Shift => (Keysym::Shift_L, Some(KeyCode::KEY_LEFTSHIFT), None),
        Key::Control => (Keysym::Control_L, Some(KeyCode::KEY_LEFTCTRL), None),
        Key::Alt => (Keysym::Alt_L, Some(KeyCode::KEY_LEFTALT), None),
        Key::Option => (Keysym::Alt_L, Some(KeyCode::KEY_LEFTALT), None),
        Key::Meta => (Keysym::Meta_L, Some(KeyCode::KEY_LEFTMETA), None),
        Key::CapsLock => (Keysym::Caps_Lock, Some(KeyCode::KEY_CAPSLOCK), None),
        Key::F1 => (Keysym::F1, Some(KeyCode::KEY_F1), None),
        Key::F2 => (Keysym::F2, Some(KeyCode::KEY_F2), None),
        Key::F3 => (Keysym::F3, Some(KeyCode::KEY_F3), None),
        Key::F4 => (Keysym::F4, Some(KeyCode::KEY_F4), None),
        Key::F5 => (Keysym::F5, Some(KeyCode::KEY_F5), None),
        Key::F6 => (Keysym::F6, Some(KeyCode::KEY_F6), None),
        Key::F7 => (Keysym::F7, Some(KeyCode::KEY_F7), None),
        Key::F8 => (Keysym::F8, Some(KeyCode::KEY_F8), None),
        Key::F9 => (Keysym::F9, Some(KeyCode::KEY_F9), None),
        Key::F10 => (Keysym::F10, Some(KeyCode::KEY_F10), None),
        Key::F11 => (Keysym::F11, Some(KeyCode::KEY_F11), None),
        Key::F12 => (Keysym::F12, Some(KeyCode::KEY_F12), None),
        _ => return Err(format!("unsupported key {key:?} on Linux")),
    }))
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
                client.button(KeyCode::BTN_LEFT, true)?;
                utils::sleep(20);
                client.button(KeyCode::BTN_LEFT, false)
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
        let Some((keysym, named_evdev, layout_char)) =
            key_to_codes(&key).map_err(anyhow::Error::msg)?
        else {
            return anyhow::Ok(());
        };

        self.with_backend(|backend| match backend {
            InputBackend::XTest => {
                let (keycode, needs_shift) = utils::keysym_to_keycode(keysym.raw())?;
                let shift = if needs_shift {
                    utils::keysym_to_keycode(xk::Shift_L).ok().map(|(code, _)| code)
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
                let resolved = match named_evdev {
                    Some(code) => Some((code, false)),
                    // Layout-aware resolution: the X/Wayland-session keymap
                    // knows where the keysym lives (X keycode = evdev + 8).
                    None => utils::keysym_to_keycode(keysym.raw())
                        .ok()
                        .filter(|(k, _)| *k >= 8)
                        .map(|(k, s)| (KeyCode::new(u16::from(k) - 8), s))
                        .or_else(|| {
                            layout_char
                                .and_then(qwerty_evdev_code)
                                .map(|code| (code, false))
                        }),
                };
                let Some((code, shift)) = resolved else {
                    return Err(anyhow::anyhow!(
                        "keysym {keysym:?} has no evdev keycode"
                    ));
                };
                if shift {
                    client.key(KeyCode::KEY_LEFTSHIFT, true)?;
                }
                client.key(code, true)?;
                utils::sleep(10);
                client.key(code, false)?;
                if shift {
                    client.key(KeyCode::KEY_LEFTSHIFT, false)?;
                }
                anyhow::Ok(())
            },
        })
    }
}
