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
use std::sync::atomic::{AtomicBool, Ordering};
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

// ---------------------------------------------------------------------------
// Absolute pointer positioning (ydotool)
// ---------------------------------------------------------------------------

/// Residual error (px) accepted before another positioning attempt is made.
const POINTER_TOLERANCE_PX: i32 = 2;
/// Residual error (px) above which the one-shot self-check warns instead of
/// logging at debug level.
const POINTER_WARN_PX: i32 = 8;
/// Pause between the corner slam and the target delta.
///
/// These must land in different compositor frames: pending relative motion is
/// coalesced into one event, so a delta sent back-to-back with the slam is
/// swallowed and the pointer stays in the corner. This was the whole reason the
/// scanner clicked into empty space on Wayland.
const POINTER_SLAM_SETTLE_MS: u32 = 40;
/// Settle time after an injected move before reading the position back.
const POINTER_SETTLE_MS: u32 = 12;
/// Attempts allowed for one move. The first one can be clamped at the screen
/// edge while the speed multiplier is still unknown, so a couple of refinements
/// are expected.
const POINTER_MAX_ATTEMPTS: u32 = 4;
/// Smallest requested axis delta that is still a usable sample for measuring
/// the compositor's pointer-speed multiplier.
const POINTER_MIN_SAMPLE_PX: i32 = 24;
/// Smallest requested delta whose absence of motion proves a stale readback.
const POINTER_MIN_STALE_DELTA_PX: i32 = 4;
/// Accepted range for a measured multiplier; anything else means the readback
/// is stale rather than that the session moves the pointer 100x.
const POINTER_SCALE_MIN: f32 = 0.2;
const POINTER_SCALE_MAX: f32 = 8.0;
/// Pointer-speed multiplier assumed before a live readback has measured it.
const POINTER_SCALE_FALLBACK: f32 = 1.0;

/// Whether this process already reported its absolute-positioning self-check.
static POINTER_SELF_CHECK_REPORTED: AtomicBool = AtomicBool::new(false);
/// Whether this process already warned that the pointer readback is frozen.
static POINTER_FROZEN_REPORTED: AtomicBool = AtomicBool::new(false);

/// Positioning state of the ydotool backend.
#[derive(Default)]
struct PointerState {
    /// Compositor pointer-speed multiplier learned from a live move.
    ///
    /// Relative pointer motion is multiplied by the session's pointer-speed
    /// setting (a "flat" profile applies a constant factor, 2.5x on the KDE
    /// session this was calibrated against), so deltas are divided by it.
    scale: Option<f32>,
    /// Last *verified* pointer position. `None` means the origin is unknown and
    /// the next move has to slam the pointer into the corner first.
    known: Option<(i32, i32)>,
}

/// Move the pointer to `(x, y)` in root coordinates.
///
/// ydotoold can only inject *relative* motion, so absolute positioning needs a
/// known origin: the pointer is slammed into the top-left corner once and every
/// later move is a relative hop from the last verified position. Three
/// environment facts shape this:
///
/// 1. The slam must be separated from the following delta by a frame, otherwise
///    the compositor coalesces them and the delta is lost (pointer stays at 0,0).
/// 2. Relative motion is multiplied by the session's pointer-speed factor, so
///    the delta has to be divided by the learned factor.
/// 3. Slamming on *every* move would be slow and would jerk the pointer to the
///    corner and back in the middle of a scroll burst, where the game needs a
///    stable hover over the grid. So the slam is only used to (re)establish the
///    origin; everything else is a small relative hop, verified each time.
///
/// The X11 pointer position is the source of truth whenever it follows injected
/// motion. A readback that does not move for a non-trivial delta is stale (the
/// game can hold the pointer); the origin is then dropped so the next move
/// re-slams, and the situation is reported once.
fn move_pointer_absolute(
    client: &mut YdotoolClient,
    x: i32,
    y: i32,
    state: &mut PointerState,
) -> anyhow::Result<()> {
    let mut last_pos: Option<(i32, i32)> = None;
    let mut stale_readback = false;
    let mut attempts_used = 0u32;

    for _ in 0..POINTER_MAX_ATTEMPTS {
        attempts_used += 1;
        let factor = state
            .scale
            .unwrap_or(POINTER_SCALE_FALLBACK)
            .clamp(POINTER_SCALE_MIN, POINTER_SCALE_MAX);
        let requested_x = ((x - state.known.unwrap_or((0, 0)).0) as f32 / factor).round() as i32;
        let requested_y = ((y - state.known.unwrap_or((0, 0)).1) as f32 / factor).round() as i32;
        let origin = match state.known {
            // Relative hop from the position verified by the previous move: no
            // slam, so the game keeps its hover and the move costs one report.
            Some(known) => {
                if requested_x != 0 || requested_y != 0 {
                    client.mouse_move_relative(requested_x, requested_y)?;
                }
                known
            },
            // Unknown origin: slam to the corner, then apply the target.
            None => {
                client.mouse_slam()?;
                utils::sleep(POINTER_SLAM_SETTLE_MS);
                client.mouse_move_relative(requested_x, requested_y)?;
                (0, 0)
            },
        };
        utils::sleep(POINTER_SETTLE_MS);

        let Some(pos) = read_pointer() else {
            break;
        };
        last_pos = Some(pos);

        if (pos.0 - x).abs() <= POINTER_TOLERANCE_PX && (pos.1 - y).abs() <= POINTER_TOLERANCE_PX {
            state.known = Some(pos);
            break;
        }
        if pos == origin
            && (requested_x.abs() >= POINTER_MIN_STALE_DELTA_PX
                || requested_y.abs() >= POINTER_MIN_STALE_DELTA_PX)
        {
            // A non-trivial delta produced no reported motion: the readback is
            // stale, so stop trusting it and re-slam on the next move.
            state.known = None;
            stale_readback = true;
            break;
        }
        // The readback is live: re-anchor on it (this absorbs a user moving the
        // physical mouse mid-scan) and refine the speed multiplier.
        if let Some(measured) = measure_pointer_scale(
            requested_x,
            requested_y,
            pos.0 - origin.0,
            pos.1 - origin.1,
        ) {
            state.scale = Some(measured);
        }
        state.known = Some(pos);
    }

    report_pointer_self_check(x, y, last_pos, state.scale, attempts_used);
    if stale_readback {
        report_frozen_readback();
    }
    Ok(())
}

/// Read the pointer position, or `None` when X11 is unavailable.
fn read_pointer() -> Option<(i32, i32)> {
    utils::query_pointer()
        .ok()
        .map(|p| (p.root_x, p.root_y))
}

/// Derive the compositor's pointer-speed multiplier from one measured move.
///
/// `origin` was the pointer position the move started from, so a live readback
/// reports `origin + delta * factor`.
fn measure_pointer_scale(dx: i32, dy: i32, observed_x: i32, observed_y: i32) -> Option<f32> {
    let (delta, observed) = if dx.abs() >= dy.abs() {
        (dx, observed_x)
    } else {
        (dy, observed_y)
    };
    if delta.abs() < POINTER_MIN_SAMPLE_PX || delta.signum() != observed.signum() {
        return None;
    }
    let measured = observed as f32 / delta as f32;
    (POINTER_SCALE_MIN..=POINTER_SCALE_MAX)
        .contains(&measured)
        .then_some(measured)
}

/// Report once that the X11 pointer does not follow injected motion.
///
/// A frozen readback means the pointer is probably not over the game window (or
/// the game holds the pointer), so clicks go somewhere else. Absolute
/// positioning cannot be verified in that state.
fn report_frozen_readback() {
    if POINTER_FROZEN_REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    log_warn!(
        "[input] 无法回读鼠标位置：注入的移动没有反映到 X11 指针上。\
         若游戏窗口未置顶，点击会落到其它窗口；请把游戏窗口切到前台后重试。",
        "[input] The pointer position cannot be read back: injected motion never reached the X11 pointer. \
         If the game window is not in front, clicks land in another window; bring it to the foreground and retry.",
    );
}

/// Report the first absolute-positioning self-check of this process.
fn report_pointer_self_check(
    x: i32,
    y: i32,
    pos: Option<(i32, i32)>,
    scale: Option<f32>,
    attempts_used: u32,
) {
    if POINTER_SELF_CHECK_REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let scale_text = match scale {
        Some(value) => format!("{value:.2}"),
        None => "?".to_string(),
    };
    match pos {
        Some(pos) if (pos.0 - x).abs().max((pos.1 - y).abs()) <= POINTER_WARN_PX => {
            log_debug!(
                "[input] ydotool 定位自检：请求 ({}, {})，实际 ({}, {})，尝试 {} 次，指针速度倍率 {}",
                "[input] ydotool positioning self-check: asked for ({}, {}), landed at ({}, {}), {} attempt(s), pointer-speed factor {}",
                x,
                y,
                pos.0,
                pos.1,
                attempts_used,
                scale_text
            );
        },
        Some(pos) => {
            log_warn!(
                "[input] ydotool 定位自检未通过：请求 ({}, {})，实际 ({}, {})，偏差 {}px（指针速度倍率 {}）。\
                 系统设置 → 鼠标 里把指针速度调回中间值会让定位更准。",
                "[input] ydotool positioning self-check failed: asked for ({}, {}), landed at ({}, {}), {}px off (pointer-speed factor {}). \
                 Moving the pointer-speed slider back to its centre in System Settings -> Mouse makes this exact.",
                x,
                y,
                pos.0,
                pos.1,
                (pos.0 - x).abs().max((pos.1 - y).abs()),
                scale_text
            );
        },
        None => {
            log_warn!(
                "[input] ydotool 定位自检：无法读取鼠标位置，请求 ({}, {}) 已按指针速度倍率 {} 补偿。\
                 若点击落到错误位置，请把系统设置 → 鼠标 的指针速度调回中间值。",
                "[input] ydotool positioning self-check: the pointer position is unreadable; ({}, {}) was injected with pointer-speed factor {}. \
                 If clicks land in the wrong place, move the pointer-speed slider back to its centre in System Settings -> Mouse.",
                x,
                y,
                scale_text
            );
        },
    }
}

pub struct LinuxControl {
    /// Lazily selected on first injected event; errors are surfaced there
    /// (the constructor mirrors the infallible Windows API).
    backend: Option<InputBackend>,
    backend_error: Option<String>,
    /// Absolute-positioning state (see [`move_pointer_absolute`]).
    pointer: PointerState,
}

impl LinuxControl {
    pub fn new() -> LinuxControl {
        LinuxControl {
            backend: None,
            backend_error: None,
            pointer: PointerState::default(),
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
        // The positioning state is moved out so the backend borrow of `self`
        // does not overlap the field borrow.
        let mut pointer = std::mem::take(&mut self.pointer);
        let result = self.with_backend(|backend| match backend {
            InputBackend::XTest => utils::fake_input(MOTION_NOTIFY_EVENT, 0, x as i16, y as i16),
            InputBackend::Ydotool(client) => move_pointer_absolute(client, x, y, &mut pointer),
        });
        self.pointer = pointer;
        result
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

    /// Scroll the wheel by `amount` detents; **positive scrolls down** (towards
    /// the end of a list).
    ///
    /// That is the convention callers get on the other platforms: enigo's
    /// Windows `mouse_scroll_y` negates the value before sending
    /// `MOUSEEVENTF_WHEEL` (`length * -120`, enigo 0.1.3), and macOS negates it
    /// explicitly. Getting this backwards is silent and nasty — every page turn
    /// scrolls the inventory *up* instead of down, so a scan past the first page
    /// re-reads the same page forever. REL_WHEEL's native sign is the opposite
    /// (positive = wheel up), hence the negations here.
    pub fn mouse_scroll(&mut self, amount: i32, _try_find: bool) -> anyhow::Result<()> {
        self.with_backend(|backend| match backend {
            InputBackend::XTest => {
                // X11 buttons: 4 = wheel up, 5 = wheel down.
                let button = if amount >= 0 { BUTTON_WHEEL_DOWN } else { BUTTON_WHEEL_UP };
                for _ in 0..amount.abs() {
                    utils::fake_input(BUTTON_PRESS_EVENT, button, 0, 0)?;
                    utils::fake_input(BUTTON_RELEASE_EVENT, button, 0, 0)?;
                }
                anyhow::Ok(())
            },
            InputBackend::Ydotool(client) => client.wheel(-amount),
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
