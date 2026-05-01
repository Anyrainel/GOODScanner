//! Windows Graphics Capture (WGC) based screen capturer.
//!
//! Two modes, selected at construction time:
//!
//! **SDR mode** (`hdr_mode = false`)
//!   Uses `B8G8R8A8UIntNormalized`. Windows delivers 8-bit sRGB pixels directly.
//!
//! **HDR mode** (`hdr_mode = true`)
//!   Uses `R16G16B16A16Float` (scRGB linear light, 1.0 = SDR reference white).
//!   A per-pixel tone curve converts FP16 values to 8-bit:
//!
//!     `out = clamp(v / white_point, 0, 1) ^ (1/2.2) × 255`
//!
//!   `white_point` is the scRGB luminance that maps to pure white (255).
//!   Values above `white_point` clip to 255; values below are scaled
//!   proportionally. A lower value produces a darker, higher-contrast image.
//!
//!   The tone curve is pre-computed into a 65536-entry u8 LUT (one entry per
//!   f16 bit pattern) so the per-pixel cost is a single table lookup.

use anyhow::{anyhow, Result};
use image::RgbImage;

use windows::core::Interface;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use crate::capture::Capturer;
use crate::positioning::{Pos, Rect};

pub struct WgcCapturer {
    d3d_device: ID3D11Device,
    d3d_context: ID3D11DeviceContext,
    frame_pool: Direct3D11CaptureFramePool,
    _session: GraphicsCaptureSession,
    frame_origin: (i32, i32),
    hdr_mode: bool,
    /// f16 bit-pattern → u8 tone-mapped value. Only populated in HDR mode.
    hdr_lut: Vec<u8>,
}

impl WgcCapturer {
    /// `white_point` is only used when `hdr_mode = true`.
    /// It sets the scRGB value that maps to 255 in the output.
    pub fn new(hwnd: isize, hdr_mode: bool, white_point: f32) -> Result<Self> {
        if hwnd == 0 {
            return Err(anyhow!("WgcCapturer: null HWND"));
        }

        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }

        let (d3d_device, d3d_context) = create_d3d11_device()?;

        let dxgi_device: IDXGIDevice = d3d_device.cast()?;
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)? };
        let rt_device: windows::Graphics::DirectX::Direct3D11::IDirect3DDevice =
            inspectable.cast()?;

        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(HWND(hwnd as *mut _))? };

        let item_size = item.Size()?;

        let pixel_format = if hdr_mode {
            DirectXPixelFormat::R16G16B16A16Float
        } else {
            DirectXPixelFormat::B8G8R8A8UIntNormalized
        };

        let frame_pool =
            Direct3D11CaptureFramePool::CreateFreeThreaded(&rt_device, pixel_format, 1, item_size)?;

        let session = frame_pool.CreateCaptureSession(&item)?;
        session.StartCapture()?;

        let mut wr = RECT::default();
        unsafe { GetWindowRect(HWND(hwnd as *mut _), &mut wr)? };
        let frame_origin = (wr.left, wr.top);

        // Build the tone-curve LUT once at construction time (~3 ms for 65536 entries).
        // In SDR mode the LUT is empty and never used.
        let hdr_lut = if hdr_mode {
            let wp = white_point.max(0.001);
            (0u32..65536)
                .map(|bits| {
                    let v = f16_to_f32(bits as u16);
                    tone_curve(v, wp)
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(Self {
            d3d_device,
            d3d_context,
            frame_pool,
            _session: session,
            frame_origin,
            hdr_mode,
            hdr_lut,
        })
    }

    fn grab_frame(&self) -> Result<RgbImage> {
        let frame = {
            let mut f = None;
            for _ in 0..20 {
                f = self.frame_pool.TryGetNextFrame().ok();
                if f.is_some() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            f.ok_or_else(|| anyhow!("WGC: no frame received within 200 ms timeout"))?
        };

        let surface = frame.Surface()?;
        let dxgi_access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let frame_tex: ID3D11Texture2D = unsafe { dxgi_access.GetInterface()? };

        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { frame_tex.GetDesc(&mut desc) };

        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: desc.Width,
            Height: desc.Height,
            MipLevels: 1,
            ArraySize: 1,
            Format: desc.Format,
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        let mut staging: Option<ID3D11Texture2D> = None;
        unsafe {
            self.d3d_device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))?
        };
        let staging = staging.ok_or_else(|| anyhow!("CreateTexture2D returned no texture"))?;
        unsafe { self.d3d_context.CopyResource(&staging, &frame_tex) };

        let mut mapped = windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.d3d_context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        }

        let width = desc.Width as u32;
        let height = desc.Height as u32;
        let pitch = mapped.RowPitch as u32;

        let img = if self.hdr_mode {
            // RGBA f16: 8 bytes per pixel. Use the pre-built LUT — one lookup per channel.
            let lut = &self.hdr_lut;
            let total = (height * pitch) as usize;
            let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, total) };
            RgbImage::from_fn(width, height, |x, y| {
                let off = (y * pitch + x * 8) as usize;
                let ri = u16::from_le_bytes([data[off], data[off + 1]]) as usize;
                let gi = u16::from_le_bytes([data[off + 2], data[off + 3]]) as usize;
                let bi = u16::from_le_bytes([data[off + 4], data[off + 5]]) as usize;
                image::Rgb([lut[ri], lut[gi], lut[bi]])
            })
        } else {
            // BGRA u8: 4 bytes per pixel
            let total = (height * pitch) as usize;
            let data = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, total) };
            RgbImage::from_fn(width, height, |x, y| {
                let off = (y * pitch + x * 4) as usize;
                image::Rgb([data[off + 2], data[off + 1], data[off]])
            })
        };

        unsafe { self.d3d_context.Unmap(&staging, 0) };

        Ok(img)
    }
}

impl Capturer<RgbImage> for WgcCapturer {
    fn capture_rect(&self, rect: Rect<i32>) -> Result<RgbImage> {
        let full = self.grab_frame()?;

        let fx = (rect.left - self.frame_origin.0).max(0) as u32;
        let fy = (rect.top - self.frame_origin.1).max(0) as u32;
        let fw = (rect.width as u32).min(full.width().saturating_sub(fx));
        let fh = (rect.height as u32).min(full.height().saturating_sub(fy));

        if fw == 0 || fh == 0 {
            return Err(anyhow!(
                "WGC: cropped rect is empty (screen rect outside captured window)"
            ));
        }

        use image::GenericImageView;
        Ok(full.view(fx, fy, fw, fh).to_image())
    }

    fn capture_color(&self, pos: Pos<i32>) -> Result<image::Rgb<u8>> {
        let full = self.grab_frame()?;
        let x = ((pos.x - self.frame_origin.0).max(0) as u32).min(full.width() - 1);
        let y = ((pos.y - self.frame_origin.1).max(0) as u32).min(full.height() - 1);
        Ok(*full.get_pixel(x, y))
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn create_d3d11_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((
        device.ok_or_else(|| anyhow!("D3D11CreateDevice: no device"))?,
        context.ok_or_else(|| anyhow!("D3D11CreateDevice: no context"))?,
    ))
}

/// Decode an IEEE 754 half-precision float to f32.
fn f16_to_f32(bits: u16) -> f32 {
    let exp = ((bits >> 10) & 0x1F) as u32;
    let mant = (bits & 0x3FF) as u32;
    let sign = (bits >> 15) as u32;
    let abs = match exp {
        0 => (mant as f32) * (1.0 / (1u32 << 24) as f32),
        31 => {
            if mant == 0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        },
        e => f32::from_bits(((e + 112) << 23) | (mant << 13)),
    };
    if sign != 0 {
        -abs
    } else {
        abs
    }
}

/// Tone-curve: clamp scRGB value `v` at `white_point`, gamma-encode to u8.
///
///   out = clamp(v / white_point, 0, 1) ^ (1/2.2) × 255
///
/// Values at or above `white_point` become 255. Values below are spread across
/// the full [0, 255] range with the standard sRGB gamma curve applied.
fn tone_curve(v: f32, white_point: f32) -> u8 {
    if v <= 0.0 || white_point <= 0.0 {
        return 0;
    }
    let t = (v / white_point).min(1.0);
    (t.powf(1.0 / 2.2) * 255.0).round() as u8
}
