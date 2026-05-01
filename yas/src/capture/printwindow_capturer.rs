use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use anyhow::{anyhow, Result};
use image::RgbImage;

use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, DeleteObject, GetDC,
    GetDIBits, ReleaseDC, SelectObject, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, SRCCOPY,
};
use windows_sys::Win32::Storage::Xps::{PrintWindow, PW_CLIENTONLY};
use windows_sys::Win32::UI::WindowsAndMessaging::{GetClientRect, PW_RENDERFULLCONTENT};

use crate::capture::Capturer;
use crate::positioning::{Pos, Rect};

pub struct PrintWindowCapturer {
    hwnd: isize,
}

impl PrintWindowCapturer {
    pub fn new(hwnd: isize) -> Result<Self> {
        if hwnd == 0 {
            return Err(anyhow!("PrintWindowCapturer: null HWND"));
        }
        Ok(Self { hwnd })
    }

    /// Capture the full client area as an RgbImage.
    fn capture_client(&self) -> Result<(RgbImage, i32, i32)> {
        let hwnd = self.hwnd as HWND;

        // Client area size
        let mut client_rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        let ok = unsafe { GetClientRect(hwnd, &mut client_rect) };
        if ok == 0 {
            return Err(anyhow!("PrintWindow: GetClientRect failed"));
        }
        let width = client_rect.right - client_rect.left;
        let height = client_rect.bottom - client_rect.top;
        if width <= 0 || height <= 0 {
            return Err(anyhow!("PrintWindow: zero-size client area"));
        }

        // Client area screen origin (for mapping screen coords to client-relative coords)
        let mut origin = POINT { x: 0, y: 0 };
        unsafe { ClientToScreen(hwnd, &mut origin) };

        let dc_window = unsafe { GetDC(null_mut()) };
        let dc_mem = unsafe { CreateCompatibleDC(dc_window) };
        let hbm = unsafe { CreateCompatibleBitmap(dc_window, width, height) };
        unsafe { SelectObject(dc_mem, hbm as *mut c_void) };

        // PW_RENDERFULLCONTENT | PW_CLIENTONLY: render DX content, client area only
        let ok = unsafe { PrintWindow(hwnd, dc_mem, PW_RENDERFULLCONTENT | PW_CLIENTONLY) };
        if ok == 0 {
            unsafe {
                DeleteObject(hbm as *mut c_void);
                DeleteObject(dc_mem as *mut c_void);
                ReleaseDC(null_mut(), dc_window);
            }
            return Err(anyhow!("PrintWindow: PrintWindow() call failed"));
        }

        let mut bitmap: BITMAP = unsafe { std::mem::zeroed() };
        unsafe {
            windows_sys::Win32::Graphics::Gdi::GetObjectW(
                hbm as *mut c_void,
                size_of::<BITMAP>() as i32,
                &mut bitmap as *mut BITMAP as *mut c_void,
            )
        };

        let mut bi: BITMAPINFOHEADER = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bitmap.bmWidth,
            biHeight: bitmap.bmHeight,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let bitmap_size = (((bitmap.bmWidth * 32 + 31) / 32) * 4 * bitmap.bmHeight) as usize;
        let mut buffer: Vec<u8> = vec![0u8; bitmap_size];

        unsafe {
            GetDIBits(
                dc_window,
                hbm,
                0,
                bitmap.bmHeight as u32,
                buffer.as_mut_ptr() as *mut c_void,
                &mut bi as *mut BITMAPINFOHEADER as *mut BITMAPINFO,
                DIB_RGB_COLORS,
            );
            DeleteObject(hbm as *mut c_void);
            DeleteObject(dc_mem as *mut c_void);
            ReleaseDC(null_mut(), dc_window);
        }

        let w = width as u32;
        let h = height as u32;
        let img = RgbImage::from_fn(w, h, |x, y| {
            // DIB rows are bottom-up
            let y_flip = h - y - 1;
            let b = buffer[((y_flip * w + x) * 4) as usize];
            let g = buffer[((y_flip * w + x) * 4 + 1) as usize];
            let r = buffer[((y_flip * w + x) * 4 + 2) as usize];
            image::Rgb([r, g, b])
        });

        Ok((img, origin.x, origin.y))
    }
}

impl Capturer<RgbImage> for PrintWindowCapturer {
    fn capture_rect(&self, rect: Rect<i32>) -> Result<RgbImage> {
        let (full, origin_x, origin_y) = self.capture_client()?;

        // Map screen rect to client-relative coords
        let cx = (rect.left - origin_x).max(0) as u32;
        let cy = (rect.top - origin_y).max(0) as u32;
        let cw = (rect.width as u32).min(full.width().saturating_sub(cx));
        let ch = (rect.height as u32).min(full.height().saturating_sub(cy));

        if cw == 0 || ch == 0 {
            return Err(anyhow!(
                "PrintWindow: cropped rect is empty (screen rect out of client area)"
            ));
        }

        use image::GenericImageView;
        Ok(full.view(cx, cy, cw, ch).to_image())
    }

    fn capture_color(&self, pos: Pos<i32>) -> Result<image::Rgb<u8>> {
        let (full, origin_x, origin_y) = self.capture_client()?;
        let x = ((pos.x - origin_x).max(0) as u32).min(full.width() - 1);
        let y = ((pos.y - origin_y).max(0) as u32).min(full.height() - 1);
        Ok(*full.get_pixel(x, y))
    }
}
