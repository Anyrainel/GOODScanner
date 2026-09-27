//! X11 helpers for Linux — the counterpart of `utils/windows.rs`.
//!
//! The game (running via Wine/Proton) is always an X11 window: either on a
//! real X11 session or on XWayland inside a Wayland session. Everything here
//! talks to `$DISPLAY` through a per-thread `x11rb` connection, mirroring the
//! Win32 helpers used on Windows:
//!
//! - window discovery by title (`enumerate_windows`, EWMH `_NET_CLIENT_LIST`
//!   first, full tree walk fallback)
//! - client-area geometry in root coordinates (`get_client_rect`)
//! - EWMH activation (`activate_window`)
//! - window liveness (`is_window_handle_valid`)
//! - global pointer state (`query_pointer`, used for RMB cancel)
//! - window-region capture (`capture_window_region` via `GetImage` ZPixmap)
//! - XTest input injection (`fake_input*`, `keysym_to_keycode`)
use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::{anyhow, Result};
use image::RgbImage;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::xproto;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, EventMask, GetImageReply, ImageFormat,
    ImageOrder, InputFocus, QueryPointerReply, Setup, StackMode, Window,
};
use x11rb::protocol::xtest;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use crate::positioning::Rect;

thread_local! {
    /// Per-thread X connection. `RustConnection` is `Send` but not `Sync`, and
    /// the scan thread, manager thread and cancel checks each need access.
    static CONN: RefCell<Option<(RustConnection, usize)>> = const { RefCell::new(None) };
    static ATOM_CACHE: RefCell<HashMap<&'static [u8], Atom>> = RefCell::new(HashMap::new());
}

/// Run `f` with the thread's X connection, connecting lazily.
pub fn with_x11<T>(f: impl FnOnce(&RustConnection, usize) -> Result<T>) -> Result<T> {
    CONN.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            let display = std::env::var("DISPLAY").unwrap_or_default();
            match x11rb::connect(None) {
                Ok((conn, screen)) => *slot = Some((conn, screen)),
                Err(e) => {
                    return Err(anyhow!(
                        "无法连接 X11 服务器（$DISPLAY={:?}）：{}。\
                         Wayland 会话需要 XWayland（游戏经 Wine/Proton 运行时通常已启用）。\n\
                         / Cannot connect to the X11 server ($DISPLAY={:?}): {}. \
                         A Wayland session requires XWayland (usually enabled when the game runs via Wine/Proton).",
                        display, e, display, e,
                    ))
                },
            }
        }
        let (conn, screen) = slot.as_ref().expect("connection was just initialized");
        f(conn, *screen)
    })
}

fn root_window(conn: &RustConnection, screen: usize) -> Window {
    conn.setup().roots[screen].root
}

/// Intern an atom once per thread (cached).
fn atom(conn: &RustConnection, name: &'static [u8]) -> Result<Atom> {
    ATOM_CACHE.with(|cache| {
        if let Some(&a) = cache.borrow().get(name) {
            return Ok(a);
        }
        let a = conn
            .intern_atom(true, name)?
            .reply()?
            .atom;
        cache.borrow_mut().insert(name, a);
        Ok(a)
    })
}

/// A visible, titled window (client window from the WM's point of view).
#[derive(Clone, Debug)]
pub struct X11WindowInfo {
    pub window: Window,
    pub title: String,
    /// Second component of WM_CLASS ("res_class"); informational only.
    pub class: String,
}

/// UTF-8 `_NET_WM_NAME`, falling back to `WM_NAME`.
fn window_title(conn: &RustConnection, window: Window) -> Option<String> {
    let net_name = atom(conn, b"_NET_WM_NAME").ok()?;
    if let Some(reply) = conn
        .get_property(false, window, net_name, AtomEnum::ANY, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        if !reply.value.is_empty() {
            if let Ok(s) = String::from_utf8(reply.value) {
                return Some(s);
            }
        }
    }
    if let Some(reply) = conn
        .get_property(false, window, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        if !reply.value.is_empty() {
            // WM_NAME is Latin-1; CJK titles only appear via _NET_WM_NAME.
            return Some(reply.value.iter().map(|&b| b as char).collect());
        }
    }
    None
}

fn window_class(conn: &RustConnection, window: Window) -> String {
    let reply = conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()
        .and_then(|c| c.reply().ok());
    match reply {
        Some(r) => {
            // Layout: "res_name\0res_class\0" — keep res_class.
            let parts: Vec<&[u8]> = r.value.split(|&b| b == 0).collect();
            parts
                .get(1)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .or_else(|| parts.first().map(|p| String::from_utf8_lossy(p).into_owned()))
                .unwrap_or_default()
        },
        None => String::new(),
    }
}

/// All client windows reported by the WM (`_NET_CLIENT_LIST`), falling back to
/// a bounded tree walk when the WM does not maintain the property.
pub fn enumerate_windows() -> Result<Vec<X11WindowInfo>> {
    with_x11(|conn, screen| {
        let root = root_window(conn, screen);

        let mut candidates: Vec<Window> = Vec::new();
        let net_client_list = atom(conn, b"_NET_CLIENT_LIST")?;
        if let Some(reply) = conn
            .get_property(false, root, net_client_list, AtomEnum::ANY, 0, 1024)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            candidates.extend(reply.value32().into_iter().flatten().collect::<Vec<u32>>());
        }
        if candidates.is_empty() {
            candidates = walk_window_tree(conn, root);
        }

        let mut out = Vec::new();
        for window in candidates {
            if let Some(title) = window_title(conn, window) {
                let title = title.trim().to_string();
                if title.is_empty() {
                    continue;
                }
                let class = window_class(conn, window);
                out.push(X11WindowInfo { window, title, class });
            }
        }
        Ok(out)
    })
}

/// BFS over the window tree, bounded to keep pathological trees cheap.
fn walk_window_tree(conn: &RustConnection, root: Window) -> Vec<Window> {
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(window) = queue.pop_front() {
        if out.len() > 2048 {
            break;
        }
        if let Some(reply) = conn.query_tree(window).ok().and_then(|c| c.reply().ok()) {
            for child in reply.children {
                out.push(child);
                queue.push_back(child);
            }
        }
    }
    out
}

/// Client-area rect in root (absolute) coordinates — the XGetGeometry +
/// TranslateCoordinates equivalent of Win32 `GetClientRect`+`ClientToScreen`.
/// A Wine game window's X window *is* the client area (WM decorations live on
/// the parent frame window).
pub fn get_client_rect(window: Window) -> Result<Rect<i32>> {
    with_x11(|conn, screen| {
        let root = root_window(conn, screen);
        let geom = conn
            .get_geometry(window)?
            .reply()
            .map_err(|e| anyhow!("GetGeometry failed: {e}"))?;
        let tr = conn
            .translate_coordinates(window, root, 0, 0)?
            .reply()
            .map_err(|e| anyhow!("TranslateCoordinates failed: {e}"))?;
        Ok(Rect::new(
            tr.dst_x as i32,
            tr.dst_y as i32,
            geom.width as i32,
            geom.height as i32,
        ))
    })
}

/// Whether the window id still refers to a live window.
pub fn is_window_handle_valid(window: Window) -> bool {
    with_x11(|conn, _| {
        Ok(conn
            .get_geometry(window)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some())
    })
    .unwrap_or(false)
}

/// Activate and raise a window (EWMH `_NET_ACTIVE_WINDOW` + XRaiseWindow +
/// SetInputFocus fallback). Under XWayland the compositor decides, so this is
/// best-effort — same contract as Win32 `ShowWindow`+`SetForegroundWindow`.
pub fn activate_window(window: Window) -> Result<()> {
    with_x11(|conn, screen| {
        let root = root_window(conn, screen);
        let net_active = atom(conn, b"_NET_ACTIVE_WINDOW")?;
        let message = ClientMessageEvent::new(
            32,
            window,
            net_active,
            [1, 0 /* timestamp */, 0 /* requestor */, 0, 0],
        );
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            message,
        )?;
        conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        conn.set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)?;
        conn.flush()?;
        Ok(())
    })
}

/// Global pointer state queried on the root window.
pub struct PointerState {
    pub root_x: i32,
    pub root_y: i32,
    /// Core button mask; bit 10 (0x400) is Button3 (right mouse button).
    pub mask: u16,
}

pub const BUTTON3_MASK: u16 = 1 << 10;

/// Query the global pointer position and button state.
pub fn query_pointer() -> Result<PointerState> {
    with_x11(|conn, screen| {
        let root = root_window(conn, screen);
        let reply: QueryPointerReply = conn
            .query_pointer(root)?
            .reply()
            .map_err(|e| anyhow!("QueryPointer failed: {e}"))?;
        Ok(PointerState {
            root_x: reply.root_x as i32,
            root_y: reply.root_y as i32,
            mask: reply.mask.into(),
        })
    })
}

/// Capture a region given in root (absolute) coordinates from the window's own
/// pixmap. Out-of-window parts of the rect come back black, matching the
/// clipping behaviour of GDI BitBlt.
pub fn capture_window_region(window: Window, rect: Rect<i32>) -> Result<RgbImage> {
    with_x11(|conn, screen| {
        let root = root_window(conn, screen);
        let geom = conn
            .get_geometry(window)?
            .reply()
            .map_err(|e| anyhow!("GetGeometry failed: {e}"))?;
        let tr = conn
            .translate_coordinates(window, root, 0, 0)?
            .reply()
            .map_err(|e| anyhow!("TranslateCoordinates failed: {e}"))?;

        // rect is absolute; clip to the window and pad the rest with black.
        let win_left = tr.dst_x as i32;
        let win_top = tr.dst_y as i32;
        let win_w = geom.width as i32;
        let win_h = geom.height as i32;

        let x0 = rect.left.saturating_sub(win_left);
        let y0 = rect.top.saturating_sub(win_top);
        let x1 = (rect.left + rect.width).saturating_sub(win_left);
        let y1 = (rect.top + rect.height).saturating_sub(win_top);

        let inter = (
            x0.max(0).min(win_w),
            y0.max(0).min(win_h),
            x1.max(0).min(win_w),
            y1.max(0).min(win_h),
        );
        if inter.2 <= inter.0 || inter.3 <= inter.1 {
            // Entirely outside — return black, like a clipped BitBlt.
            return Ok(RgbImage::from_pixel(
                rect.width.max(0) as u32,
                rect.height.max(0) as u32,
                image::Rgb([0, 0, 0]),
            ));
        }

        let iw = (inter.2 - inter.0) as u16;
        let ih = (inter.3 - inter.1) as u16;
        let reply: GetImageReply = conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                window,
                inter.0 as i16,
                inter.1 as i16,
                iw,
                ih,
                u32::MAX,
            )?
            .reply()
            .map_err(|e| anyhow!("GetImage failed: {e}"))?;

        let captured = decode_zpixmap(
            &reply.data,
            reply.depth,
            iw,
            ih,
            conn.setup().image_byte_order,
        )?;

        if iw as i32 == rect.width && ih as i32 == rect.height {
            return Ok(captured);
        }

        // Blit the captured intersection into a black canvas.
        let mut canvas = RgbImage::from_pixel(
            rect.width.max(0) as u32,
            rect.height.max(0) as u32,
            image::Rgb([0, 0, 0]),
        );
        let paste_x = (x0 - inter.0).max(0) as i64;
        let paste_y = (y0 - inter.1).max(0) as i64;
        for dy in 0..ih as i64 {
            for dx in 0..iw as i64 {
                let px = captured.get_pixel(dx as u32, dy as u32);
                canvas.put_pixel((paste_x + dx) as u32, (paste_y + dy) as u32, *px);
            }
        }
        Ok(canvas)
    })
}

/// ZPixmap (depth 24/32) → RGB. Server byte order decides component layout.
fn decode_zpixmap(
    data: &[u8],
    depth: u8,
    width: u16,
    height: u16,
    byte_order: ImageOrder,
) -> Result<RgbImage> {
    let n = width as usize * height as usize;
    let lsb = byte_order == ImageOrder::LSB_FIRST;
    let mut rgb = Vec::with_capacity(n * 3);

    if data.len() >= n * 4 {
        for px in data.chunks_exact(4).take(n) {
            // 32bpp: X stores B,G,R,X (LSB) or X,R,G,B (MSB).
            let (r, g, b) = if lsb { (px[2], px[1], px[0]) } else { (px[1], px[2], px[3]) };
            rgb.extend_from_slice(&[r, g, b]);
        }
    } else if data.len() >= n * 3 {
        for px in data.chunks_exact(3).take(n) {
            let (r, g, b) = if lsb { (px[2], px[1], px[0]) } else { (px[0], px[1], px[2]) };
            rgb.extend_from_slice(&[r, g, b]);
        }
    } else {
        return Err(anyhow!(
            "unsupported ZPixmap payload: depth={depth}, {} bytes for {width}x{height}",
            data.len()
        ));
    }

    RgbImage::from_raw(width as u32, height as u32, rgb)
        .ok_or_else(|| anyhow!("ZPixmap size mismatch"))
}

// ---------------------------------------------------------------------------
// XTest input injection
// ---------------------------------------------------------------------------

/// Whether the X server provides the XTEST extension (required for input
/// injection; XWayland implements it).
pub fn xtest_available() -> bool {
    with_x11(|conn, _| {
        conn.extension_information(xtest::X11_EXTENSION_NAME)
            .map(|info| info.is_some())
            .map_err(|e| anyhow!("extension query failed: {e}"))
    })
    .unwrap_or(false)
}

/// Send one XTEST FakeInput request. `type_` is a core event type
/// (2=KeyPress, 3=KeyRelease, 4=ButtonPress, 5=ButtonRelease, 6=MotionNotify);
/// `detail` is keycode / button number / 0.
pub fn fake_input(type_: u8, detail: u8, root_x: i16, root_y: i16) -> Result<()> {
    with_x11(|conn, screen| {
        if conn.extension_information(xtest::X11_EXTENSION_NAME)?.is_none() {
            return Err(anyhow!(
                "X 服务器不支持 XTEST 扩展，无法模拟输入。\n\
                 / The X server does not provide the XTEST extension; input cannot be injected."
            ));
        }
        let root = root_window(conn, screen);
        xtest::fake_input(conn, type_, detail, 0, root, root_x, root_y, 0)?.check()?;
        conn.flush()?;
        Ok(())
    })
}

/// Whether XTEST fake motion actually moves the pointer. On Wayland sessions
/// most compositors (KWin, Mutter) advertise the extension but ignore fake
/// input, so callers use this to decide between the XTEST and ydotool
/// backends.
pub fn xtest_effective() -> bool {
    let check = |dx: i32| -> bool {
        match query_pointer() {
            Ok(p) => {
                let target = p.root_x + dx;
                let ok = fake_input(xproto::MOTION_NOTIFY_EVENT, 0, target as i16, p.root_y as i16)
                    .is_ok();
                if !ok {
                    return false;
                }
                std::thread::sleep(std::time::Duration::from_millis(30));
                matches!(query_pointer(), Ok(q) if q.root_x == target)
            },
            Err(_) => false,
        }
    };
    check(1) || check(-1)
}

/// Map a keysym to (keycode, needs_shift) using the server's keymap.
pub fn keysym_to_keycode(keysym: u32) -> Result<(u8, bool)> {
    with_x11(|conn, _| {
        let setup: &Setup = conn.setup();
        let min = setup.min_keycode;
        let max = setup.max_keycode;
        if max < min {
            return Err(anyhow!("invalid keycode range"));
        }
        let count = max - min + 1;
        let reply = conn
            .get_keyboard_mapping(min, count)?
            .reply()
            .map_err(|e| anyhow!("GetKeyboardMapping failed: {e}"))?;
        let per = reply.keysyms_per_keycode as usize;
        if per == 0 {
            return Err(anyhow!("empty keyboard mapping"));
        }
        for (i, chunk) in reply.keysyms.chunks(per).enumerate() {
            // Level 0 = unshifted, level 1 = shifted.
            if chunk.first() == Some(&keysym) {
                return Ok((min + i as u8, false));
            }
            if chunk.get(1) == Some(&keysym) {
                return Ok((min + i as u8, true));
            }
        }
        Err(anyhow!("keysym {keysym:#x} not found in keymap"))
    })
}
