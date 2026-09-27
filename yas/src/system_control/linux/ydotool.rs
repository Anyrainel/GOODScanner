//! Minimal client for the ydotoold (v1.0.x) unix socket.
//!
//! On Wayland sessions the compositor ignores XTEST fake input, so injection
//! must go through uinput. ydotoold holds the privileged /dev/uinput device
//! and forwards raw `struct input_event`s received on its socket — there is
//! no handshake in the 1.0 protocol, clients just `write()` events.
//!
//! Layout of `struct input_event` on 64-bit Linux (what ydotoold's
//! `recv(fd, &uev, sizeof(uev))` expects): 8B sec + 8B usec + u16 type +
//! u16 code + i32 value = 24 bytes, host endianness, timestamps ignored.
//!
//! Absolute positioning uses ydotool's own trick: slam the cursor to the
//! top-left corner with an INT32_MIN relative delta (libinput clamps it),
//! then apply +x/+y deltas. This requires pointer acceleration to be
//! disabled/flat (ydotoold itself tries `xinput` to arrange this on X11).

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use evdev::KeyCode;

// evdev event/type codes (protocol level; key codes come from `evdev`).
pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const SYN_REPORT: u16 = 0x00;
pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_WHEEL: u16 = 0x08;

const INT32_MIN: i32 = i32::MIN;

/// Candidate socket paths, in ydotool's own resolution order.
fn socket_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(p) = std::env::var("YDOTOOL_SOCKET") {
        if !p.is_empty() {
            paths.push(PathBuf::from(p));
        }
    }
    // Arch/systemd user service ships YDOTOOL_SOCKET=$XDG_RUNTIME_DIR/ydotool.socket
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !dir.is_empty() {
            paths.push(PathBuf::from(dir).join("ydotool.socket"));
        }
    }
    paths.push(PathBuf::from("/run/ydotoold.socket"));
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(home).join(".ydotool.socket"));
    }
    paths
}

pub struct YdotoolClient {
    stream: UnixStream,
}

impl YdotoolClient {
    pub fn connect() -> Result<Self> {
        let mut last_err = String::from("no candidate socket exists");
        for path in socket_candidates() {
            match UnixStream::connect(&path) {
                Ok(stream) => return Ok(Self { stream }),
                Err(e) => last_err = format!("{}: {}", path.display(), e),
            }
        }
        Err(anyhow!(
            "无法连接 ydotoold（{}）。\
             Wayland 会话需要 ydotool 来模拟鼠标键盘：安装 ydotool 并启用其服务\
             （Arch 系: sudo pacman -S ydotool && systemctl --user enable --now ydotool），\
             或改用 X11 会话。\n\
             / Cannot connect to ydotoold ({}). A Wayland session needs ydotool for input \
             injection: install it and enable the service (Arch: sudo pacman -S ydotool && \
             systemctl --user enable --now ydotool), or use an X11 session instead.",
            last_err, last_err,
        ))
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<()> {
        let mut buf = [0u8; 24];
        buf[16..18].copy_from_slice(&type_.to_ne_bytes());
        buf[18..20].copy_from_slice(&code.to_ne_bytes());
        buf[20..24].copy_from_slice(&value.to_ne_bytes());
        self.stream.write_all(&buf)?;
        Ok(())
    }

    fn syn(&mut self) -> Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }

    /// Move the pointer to absolute screen coordinates (corner-slam + delta).
    pub fn mouse_move_abs(&mut self, x: i32, y: i32) -> Result<()> {
        self.emit(EV_REL, REL_X, INT32_MIN)?;
        self.emit(EV_REL, REL_Y, INT32_MIN)?;
        self.syn()?;
        self.emit(EV_REL, REL_X, x)?;
        self.emit(EV_REL, REL_Y, y)?;
        self.syn()
    }

    /// Press (1) or release (0) a button.
    pub fn button(&mut self, btn: KeyCode, press: bool) -> Result<()> {
        self.emit(EV_KEY, btn.code(), i32::from(press))?;
        self.syn()
    }

    /// Scroll by `notches` detents (positive = up).
    pub fn wheel(&mut self, notches: i32) -> Result<()> {
        self.emit(EV_REL, REL_WHEEL, notches)?;
        self.syn()
    }

    /// Press (1) or release (0) a key.
    pub fn key(&mut self, code: KeyCode, press: bool) -> Result<()> {
        self.emit(EV_KEY, code.code(), i32::from(press))?;
        self.syn()
    }
}

/// US-QWERTY fallback for characters when no live keymap is available to ask
/// (evdev keycodes are physical-key codes, so char→code is layout knowledge).
pub fn qwerty_evdev_code(c: char) -> Option<KeyCode> {
    Some(match c.to_ascii_lowercase() {
        '1' => KeyCode::KEY_1,
        '2' => KeyCode::KEY_2,
        '3' => KeyCode::KEY_3,
        '4' => KeyCode::KEY_4,
        '5' => KeyCode::KEY_5,
        '6' => KeyCode::KEY_6,
        '7' => KeyCode::KEY_7,
        '8' => KeyCode::KEY_8,
        '9' => KeyCode::KEY_9,
        '0' => KeyCode::KEY_0,
        '-' => KeyCode::KEY_MINUS,
        '=' => KeyCode::KEY_EQUAL,
        'q' => KeyCode::KEY_Q,
        'w' => KeyCode::KEY_W,
        'e' => KeyCode::KEY_E,
        'r' => KeyCode::KEY_R,
        't' => KeyCode::KEY_T,
        'y' => KeyCode::KEY_Y,
        'u' => KeyCode::KEY_U,
        'i' => KeyCode::KEY_I,
        'o' => KeyCode::KEY_O,
        'p' => KeyCode::KEY_P,
        '[' => KeyCode::KEY_LEFTBRACE,
        ']' => KeyCode::KEY_RIGHTBRACE,
        'a' => KeyCode::KEY_A,
        's' => KeyCode::KEY_S,
        'd' => KeyCode::KEY_D,
        'f' => KeyCode::KEY_F,
        'g' => KeyCode::KEY_G,
        'h' => KeyCode::KEY_H,
        'j' => KeyCode::KEY_J,
        'k' => KeyCode::KEY_K,
        'l' => KeyCode::KEY_L,
        ';' => KeyCode::KEY_SEMICOLON,
        '\'' => KeyCode::KEY_APOSTROPHE,
        '`' => KeyCode::KEY_GRAVE,
        '\\' => KeyCode::KEY_BACKSLASH,
        'z' => KeyCode::KEY_Z,
        'x' => KeyCode::KEY_X,
        'c' => KeyCode::KEY_C,
        'v' => KeyCode::KEY_V,
        'b' => KeyCode::KEY_B,
        'n' => KeyCode::KEY_N,
        'm' => KeyCode::KEY_M,
        ',' => KeyCode::KEY_COMMA,
        '.' => KeyCode::KEY_DOT,
        '/' => KeyCode::KEY_SLASH,
        ' ' => KeyCode::KEY_SPACE,
        _ => return None,
    })
}
