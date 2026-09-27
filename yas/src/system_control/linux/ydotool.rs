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

// evdev event/type codes.
pub const EV_SYN: u16 = 0x00;
pub const EV_KEY: u16 = 0x01;
pub const EV_REL: u16 = 0x02;
pub const SYN_REPORT: u16 = 0x00;
pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
pub const REL_WHEEL: u16 = 0x08;

// Buttons (evdev BTN_*).
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;

// Common evdev key codes (linux/input-event-codes.h).
pub const KEY_ESC: u16 = 1;
pub const KEY_1: u16 = 2;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_Q: u16 = 16;
pub const KEY_E: u16 = 18;
pub const KEY_R: u16 = 19;
pub const KEY_T: u16 = 20;
pub const KEY_Y: u16 = 21;
pub const KEY_U: u16 = 22;
pub const KEY_I: u16 = 23;
pub const KEY_O: u16 = 24;
pub const KEY_P: u16 = 25;
pub const KEY_LEFTBRACE: u16 = 26;
pub const KEY_RIGHTBRACE: u16 = 27;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_A: u16 = 30;
pub const KEY_S: u16 = 31;
pub const KEY_D: u16 = 32;
pub const KEY_F: u16 = 33;
pub const KEY_G: u16 = 34;
pub const KEY_H: u16 = 35;
pub const KEY_J: u16 = 36;
pub const KEY_K: u16 = 37;
pub const KEY_L: u16 = 38;
pub const KEY_SEMICOLON: u16 = 39;
pub const KEY_APOSTROPHE: u16 = 40;
pub const KEY_GRAVE: u16 = 41;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_BACKSLASH: u16 = 43;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_N: u16 = 49;
pub const KEY_M: u16 = 50;
pub const KEY_COMMA: u16 = 51;
pub const KEY_DOT: u16 = 52;
pub const KEY_SLASH: u16 = 53;
pub const KEY_SPACE: u16 = 57;
pub const KEY_CAPSLOCK: u16 = 58;
pub const KEY_F1: u16 = 59;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_DELETE: u16 = 111;
pub const KEY_LEFTALT: u16 = 56;

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
    pub fn button(&mut self, btn: u16, press: bool) -> Result<()> {
        self.emit(EV_KEY, btn, i32::from(press))?;
        self.syn()
    }

    /// Scroll by `notches` detents (positive = up).
    pub fn wheel(&mut self, notches: i32) -> Result<()> {
        self.emit(EV_REL, REL_WHEEL, notches)?;
        self.syn()
    }

    /// Press (1) or release (0) a key.
    pub fn key(&mut self, code: u16, press: bool) -> Result<()> {
        self.emit(EV_KEY, code, i32::from(press))?;
        self.syn()
    }
}

/// Map an ASCII character (unshifted, latin layout) to an evdev keycode.
pub fn char_to_evdev_code(c: char) -> Option<u16> {
    Some(match c.to_ascii_lowercase() {
        '1' => KEY_1,
        '2'..='9' => KEY_1 + (c as u16 - '2' as u16) + 1,
        '0' => KEY_0,
        '-' => KEY_MINUS,
        '=' => KEY_EQUAL,
        'q' => KEY_Q,
        'w' => KEY_Q + 1,
        'e' => KEY_E,
        'r' => KEY_R,
        't' => KEY_T,
        'y' => KEY_Y,
        'u' => KEY_U,
        'i' => KEY_I,
        'o' => KEY_O,
        'p' => KEY_P,
        '[' => KEY_LEFTBRACE,
        ']' => KEY_RIGHTBRACE,
        'a' => KEY_A,
        's' => KEY_S,
        'd' => KEY_D,
        'f' => KEY_F,
        'g' => KEY_G,
        'h' => KEY_H,
        'j' => KEY_J,
        'k' => KEY_K,
        'l' => KEY_L,
        ';' => KEY_SEMICOLON,
        '\'' => KEY_APOSTROPHE,
        '`' => KEY_GRAVE,
        '\\' => KEY_BACKSLASH,
        'z' => KEY_Z,
        'x' => KEY_X,
        'c' => KEY_C,
        'v' => KEY_V,
        'b' => KEY_B,
        'n' => KEY_N,
        'm' => KEY_M,
        ',' => KEY_COMMA,
        '.' => KEY_DOT,
        '/' => KEY_SLASH,
        ' ' => KEY_SPACE,
        _ => return None,
    })
}
