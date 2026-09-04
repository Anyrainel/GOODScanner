use std::{collections::VecDeque, rc::Rc, time::Duration};

use enigo::Key;
use image::RgbImage;
use yas::{
    cancel::CancelToken,
    capture::{CaptureMethod, Capturer, GenericCapturer},
    positioning::Rect,
    system_control::SystemControl,
};

use crate::{
    error::{hints, HsrError, HsrResult},
    vision::Point,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowIdentity {
    pub title: String,
    pub width: u32,
    pub height: u32,
    #[cfg(target_os = "windows")]
    hwnd: isize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputCommand {
    Click(Point),
    Key(char),
    /// One or more vertical mouse-wheel detents. Positive values scroll the
    /// inventory down, matching the existing YAS Windows controller contract.
    Scroll(i32),
    Escape,
}

/// Minimal screenshot/controller boundary shared by the real Windows runtime
/// and deterministic controller simulations.
pub trait HsrDevice {
    fn identity(&self) -> &WindowIdentity;
    fn capture_client(&mut self) -> HsrResult<RgbImage>;
    fn focus_and_verify(&mut self) -> HsrResult<()>;
    fn is_foreground(&self) -> bool;
    fn input(&mut self, command: InputCommand) -> HsrResult<()>;
    fn wait(&mut self, duration: Duration) -> HsrResult<()>;
    fn is_cancelled(&self) -> bool;
}

pub struct WindowsHsrDevice {
    identity: WindowIdentity,
    client_rect: Rect<i32>,
    verified_frame_dimensions: Option<(u32, u32)>,
    capturer: Rc<dyn Capturer<RgbImage>>,
    control: SystemControl,
    cancel: CancelToken,
}

impl WindowsHsrDevice {
    pub fn locate(capture_method: CaptureMethod) -> HsrResult<Self> {
        Self::locate_with_cancel(capture_method, CancelToken::new())
    }

    /// Locate the HSR client and bind the supplied per-run cancellation token
    /// to every wait and input operation performed by the device.
    pub fn locate_with_cancel(
        capture_method: CaptureMethod,
        cancel: CancelToken,
    ) -> HsrResult<Self> {
        if cancel.is_cancelled() {
            return Err(HsrError::new(
                "HSR-DEVICE-CANCELLED",
                hints::CANCELLED,
                "cancellation token was already active before HSR device discovery",
            ));
        }
        #[cfg(target_os = "windows")]
        {
            let (hwnd, title, client_rect) = locate_hsr_window()?;
            let capturer: Rc<dyn Capturer<RgbImage>> = match capture_method {
                CaptureMethod::BitBlt => Rc::new(GenericCapturer::new().map_err(device_error)?),
                CaptureMethod::PrintWindow => {
                    Rc::new(yas::capture::PrintWindowCapturer::new(hwnd).map_err(device_error)?)
                },
                CaptureMethod::Wgc => Rc::new(
                    yas::capture::WgcCapturer::new(hwnd, false, 203.0).map_err(device_error)?,
                ),
            };
            Ok(Self {
                identity: WindowIdentity {
                    title,
                    width: client_rect.width as u32,
                    height: client_rect.height as u32,
                    hwnd,
                },
                client_rect,
                verified_frame_dimensions: None,
                capturer,
                control: SystemControl::new(),
                cancel,
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (capture_method, cancel);
            Err(HsrError::new(
                "HSR-DEVICE-PLATFORM",
                hints::DEVICE_UNAVAILABLE,
                "live HSR capture is currently implemented for Windows only",
            ))
        }
    }

    fn refresh_rect(&mut self) -> HsrResult<()> {
        #[cfg(target_os = "windows")]
        {
            let rect =
                yas::utils::get_client_rect(self.identity.hwnd as _).map_err(device_error)?;
            if rect.width <= 0 || rect.height <= 0 {
                return Err(HsrError::new(
                    "HSR-DEVICE-GEOMETRY",
                    hints::DEVICE_UNAVAILABLE,
                    format!("invalid client geometry={}x{}", rect.width, rect.height),
                ));
            }
            validate_aspect(rect.width as u32, rect.height as u32)?;
            self.client_rect = rect;
            self.identity.width = rect.width as u32;
            self.identity.height = rect.height as u32;
            self.verified_frame_dimensions = None;
        }
        Ok(())
    }

    fn revalidate_rect(&self) -> HsrResult<Rect<i32>> {
        #[cfg(target_os = "windows")]
        {
            let current =
                yas::utils::get_client_rect(self.identity.hwnd as _).map_err(device_error)?;
            validate_client_geometry(self.client_rect, current, self.verified_frame_dimensions)?;
            Ok(current)
        }
        #[cfg(not(target_os = "windows"))]
        Ok(self.client_rect)
    }

    fn ensure_input_attended(&self) -> HsrResult<()> {
        if self.cancel.check_rmb() {
            return Err(HsrError::new(
                "HSR-DEVICE-CANCELLED",
                hints::CANCELLED,
                "right-mouse emergency abort or cancellation token is active",
            ));
        }
        if !self.is_foreground() {
            return Err(HsrError::new(
                "HSR-DEVICE-FOCUS",
                hints::FOCUS_REQUIRED,
                "input refused because the selected HSR client is not foreground",
            ));
        }
        Ok(())
    }
}

impl HsrDevice for WindowsHsrDevice {
    fn identity(&self) -> &WindowIdentity {
        &self.identity
    }

    fn capture_client(&mut self) -> HsrResult<RgbImage> {
        let rect = self.revalidate_rect()?;
        let image = self
            .capturer
            .capture_rect(self.client_rect)
            .map_err(device_error)?;
        let expected = (rect.width as u32, rect.height as u32);
        if image.dimensions() != expected {
            return Err(HsrError::new(
                "HSR-DEVICE-FRAME-GEOMETRY",
                hints::SCREEN_INVALID,
                format!(
                    "captured frame dimensions={:?} do not match verified client dimensions={expected:?}",
                    image.dimensions()
                ),
            ));
        }
        self.verified_frame_dimensions = Some(expected);
        Ok(image)
    }

    fn focus_and_verify(&mut self) -> HsrResult<()> {
        #[cfg(target_os = "windows")]
        {
            if !yas::utils::is_window_handle_valid(self.identity.hwnd as _) {
                return Err(HsrError::new(
                    "HSR-DEVICE-LOST",
                    hints::DEVICE_UNAVAILABLE,
                    "the selected HSR window handle is no longer valid",
                ));
            }
            let focused = yas::utils::show_window_and_set_foreground(self.identity.hwnd as _)
                .map_err(device_error)?;
            if !focused || !self.is_foreground() {
                return Err(HsrError::new(
                    "HSR-DEVICE-FOCUS",
                    hints::FOCUS_REQUIRED,
                    "Windows did not grant foreground focus to the selected HSR client",
                ));
            }
            self.refresh_rect()?;
        }
        Ok(())
    }

    fn is_foreground(&self) -> bool {
        #[cfg(target_os = "windows")]
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as isize
                == self.identity.hwnd
        }
        #[cfg(not(target_os = "windows"))]
        false
    }

    fn input(&mut self, command: InputCommand) -> HsrResult<()> {
        self.ensure_input_attended()?;
        match command {
            InputCommand::Click(point) => {
                if !(0.0..=1.0).contains(&point.x) || !(0.0..=1.0).contains(&point.y) {
                    return Err(HsrError::new(
                        "HSR-DEVICE-INPUT",
                        hints::SCREEN_INVALID,
                        format!(
                            "normalized click outside client: ({:.4},{:.4})",
                            point.x, point.y
                        ),
                    ));
                }
                // Bind the normalized point to the exact client rectangle that
                // produced the last verified frame (or the focused baseline
                // before the first frame). A moved/resized window invalidates
                // the click instead of silently reusing stale coordinates.
                let rect = self.revalidate_rect()?;
                let x = rect.left + (point.x * rect.width.saturating_sub(1) as f64).round() as i32;
                let y = rect.top + (point.y * rect.height.saturating_sub(1) as f64).round() as i32;
                self.control.mouse_move_to(x, y).map_err(device_error)?;
                std::thread::sleep(Duration::from_millis(12));

                // Close the cursor-move/click TOCTOU window: a focus loss,
                // emergency cancel, or geometry drift during the delay must
                // prevent the actual mouse-button event.
                self.ensure_input_attended()?;
                self.revalidate_rect()?;
                self.control.mouse_click().map_err(device_error)?;
            },
            InputCommand::Key(character) => self
                .control
                .key_press(Key::Layout(character))
                .map_err(device_error)?,
            InputCommand::Scroll(amount) => {
                if amount == 0 || amount.unsigned_abs() > 120 {
                    return Err(HsrError::new(
                        "HSR-DEVICE-INPUT",
                        hints::SCREEN_INVALID,
                        format!("invalid mouse-wheel detent count={amount}; allowed=1..=120"),
                    ));
                }
                #[cfg(any(target_os = "windows", target_os = "linux"))]
                self.control
                    .mouse_scroll(amount, false)
                    .map_err(device_error)?;
                #[cfg(target_os = "macos")]
                self.control.mouse_scroll(amount).map_err(device_error)?;
            },
            InputCommand::Escape => self.control.key_press(Key::Escape).map_err(device_error)?,
        }
        Ok(())
    }

    fn wait(&mut self, duration: Duration) -> HsrResult<()> {
        let mut remaining = duration;
        while !remaining.is_zero() {
            if self.cancel.check_rmb() {
                return Err(HsrError::new(
                    "HSR-DEVICE-CANCELLED",
                    hints::CANCELLED,
                    "right-mouse emergency abort or cancellation token is active",
                ));
            }
            let step = remaining.min(Duration::from_millis(20));
            std::thread::sleep(step);
            remaining = remaining.saturating_sub(step);
        }
        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

fn validate_client_geometry(
    expected: Rect<i32>,
    current: Rect<i32>,
    verified_frame_dimensions: Option<(u32, u32)>,
) -> HsrResult<()> {
    if current.width <= 0 || current.height <= 0 {
        return Err(HsrError::new(
            "HSR-DEVICE-GEOMETRY",
            hints::DEVICE_UNAVAILABLE,
            format!(
                "invalid current client geometry={}x{}",
                current.width, current.height
            ),
        ));
    }
    validate_aspect(current.width as u32, current.height as u32)?;
    if current != expected {
        return Err(HsrError::new(
            "HSR-DEVICE-GEOMETRY-DRIFT",
            hints::SCREEN_INVALID,
            format!("client moved or resized; verified={expected}, current={current}"),
        ));
    }
    if let Some(frame_dimensions) = verified_frame_dimensions {
        let current_dimensions = (current.width as u32, current.height as u32);
        if current_dimensions != frame_dimensions {
            return Err(HsrError::new(
                "HSR-DEVICE-GEOMETRY-DRIFT",
                hints::SCREEN_INVALID,
                format!(
                    "client dimensions={current_dimensions:?} differ from verified frame dimensions={frame_dimensions:?}"
                ),
            ));
        }
    }
    Ok(())
}

fn validate_aspect(width: u32, height: u32) -> HsrResult<()> {
    if width < 960 || height < 540 {
        return Err(HsrError::new(
            "HSR-DEVICE-SIZE",
            hints::SCREEN_INVALID,
            format!("client resolution {width}x{height} is below the tested minimum 960x540"),
        ));
    }
    let aspect = width as f64 / height as f64;
    if (aspect - 16.0 / 9.0).abs() > 0.015 {
        return Err(HsrError::new(
            "HSR-DEVICE-ASPECT",
            hints::SCREEN_INVALID,
            format!("client resolution={width}x{height}; expected a 16:9 client area"),
        ));
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn locate_hsr_window() -> HsrResult<(isize, String, Rect<i32>)> {
    yas::utils::set_dpi_awareness();
    const TITLES: &[&str] = &[
        "Honkai: Star Rail",
        "崩坏：星穹铁道",
        "崩壞：星穹鐵道",
        "붕괴:\u{00a0}스타레일",
        "崩壊：スターレイル",
        "Honkai\u{00a0}: Star Rail",
    ];
    let mut matches = Vec::new();
    for hwnd in yas::utils::iterate_window() {
        let Some(title) = yas::utils::get_window_title(hwnd) else {
            continue;
        };
        if !TITLES.iter().any(|candidate| title.trim() == *candidate) {
            continue;
        }
        let class = window_class(hwnd);
        if class == "UnityWndClass" || class == "Qt5152QWindowIcon" {
            let rect = yas::utils::get_client_rect(hwnd).map_err(device_error)?;
            matches.push((hwnd as isize, title.trim().to_string(), rect));
        }
    }
    match matches.len() {
        0 => Err(HsrError::new(
            "HSR-DEVICE-NOT-FOUND",
            hints::DEVICE_UNAVAILABLE,
            "no exact HSR game window with a supported window class was found",
        )),
        1 => {
            let result = matches.remove(0);
            validate_aspect(result.2.width as u32, result.2.height as u32)?;
            Ok(result)
        },
        count => Err(HsrError::new(
            "HSR-DEVICE-AMBIGUOUS",
            hints::DEVICE_UNAVAILABLE,
            format!("found {count} matching HSR game windows; close duplicates and retry"),
        )),
    }
}

#[cfg(target_os = "windows")]
fn window_class(hwnd: windows_sys::Win32::Foundation::HWND) -> String {
    use std::os::windows::ffi::OsStringExt;
    let mut buffer = [0_u16; 256];
    let length = unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
            hwnd,
            buffer.as_mut_ptr(),
            buffer.len() as i32,
        )
    };
    std::ffi::OsString::from_wide(&buffer[..length.max(0) as usize])
        .to_string_lossy()
        .into_owned()
}

fn device_error(error: impl std::fmt::Display) -> HsrError {
    HsrError::new(
        "HSR-DEVICE-IO",
        hints::DEVICE_UNAVAILABLE,
        format!("device I/O failed; cause={error}"),
    )
}

/// Deterministic controller used by integration tests. It returns scripted
/// frames and records every command issued by the production scanner.
pub struct ReplayDevice {
    identity: WindowIdentity,
    frames: VecDeque<RgbImage>,
    commands: Vec<InputCommand>,
    foreground: bool,
    cancelled: bool,
    lose_focus_after_commands: Option<usize>,
    cancel_after_commands: Option<usize>,
    geometry_drift_after_commands: Option<(usize, u32, u32)>,
    geometry_drifted: bool,
    verified_frame_dimensions: Option<(u32, u32)>,
    lose_focus_during_next_click: bool,
    cancel_during_next_click: bool,
}

impl ReplayDevice {
    pub fn new(width: u32, height: u32, frames: Vec<RgbImage>) -> Self {
        Self {
            identity: WindowIdentity {
                title: "Sanitized HSR replay".to_string(),
                width,
                height,
                #[cfg(target_os = "windows")]
                hwnd: 0,
            },
            frames: frames.into(),
            commands: Vec::new(),
            foreground: true,
            cancelled: false,
            lose_focus_after_commands: None,
            cancel_after_commands: None,
            geometry_drift_after_commands: None,
            geometry_drifted: false,
            verified_frame_dimensions: None,
            lose_focus_during_next_click: false,
            cancel_during_next_click: false,
        }
    }

    pub fn commands(&self) -> &[InputCommand] {
        &self.commands
    }

    pub fn remaining_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn set_foreground(&mut self, foreground: bool) {
        self.foreground = foreground;
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    /// Test hook that models the game losing foreground ownership immediately
    /// after the requested number of accepted controller commands.
    pub fn lose_focus_after_commands(&mut self, count: usize) {
        self.lose_focus_after_commands = Some(count);
    }

    /// Test hook that models an emergency cancellation immediately after the
    /// requested number of accepted controller commands.
    pub fn cancel_after_commands(&mut self, count: usize) {
        self.cancel_after_commands = Some(count);
    }

    /// Test hook for a move/resize after an accepted command but before the
    /// next click. Equal dimensions model a same-size move.
    pub fn drift_geometry_after_commands(&mut self, count: usize, width: u32, height: u32) {
        self.geometry_drift_after_commands = Some((count, width, height));
    }

    /// Test hook for a foreground loss during cursor movement, immediately
    /// before the next simulated button event.
    pub fn lose_focus_during_next_click(&mut self) {
        self.lose_focus_during_next_click = true;
    }

    /// Test hook for cancellation during cursor movement, immediately before
    /// the next simulated button event.
    pub fn cancel_during_next_click(&mut self) {
        self.cancel_during_next_click = true;
    }
}

impl HsrDevice for ReplayDevice {
    fn identity(&self) -> &WindowIdentity {
        &self.identity
    }

    fn capture_client(&mut self) -> HsrResult<RgbImage> {
        if self.geometry_drifted {
            return Err(HsrError::new(
                "HSR-REPLAY-GEOMETRY",
                hints::SCREEN_INVALID,
                "scripted client geometry drifted after its verified frame",
            ));
        }
        let frame = self.frames.pop_front().ok_or_else(|| {
            HsrError::new(
                "HSR-REPLAY-EOF",
                hints::SCREEN_INVALID,
                "scripted frame queue exhausted",
            )
        })?;
        let dimensions = frame.dimensions();
        let identity_dimensions = (self.identity.width, self.identity.height);
        if dimensions != identity_dimensions {
            return Err(HsrError::new(
                "HSR-REPLAY-GEOMETRY",
                hints::SCREEN_INVALID,
                format!(
                    "scripted frame dimensions={dimensions:?} differ from identity dimensions={identity_dimensions:?}"
                ),
            ));
        }
        self.verified_frame_dimensions = Some(dimensions);
        Ok(frame)
    }

    fn focus_and_verify(&mut self) -> HsrResult<()> {
        if self.foreground {
            Ok(())
        } else {
            Err(HsrError::new(
                "HSR-REPLAY-FOCUS",
                hints::FOCUS_REQUIRED,
                "scripted device is not foreground",
            ))
        }
    }

    fn is_foreground(&self) -> bool {
        self.foreground
    }

    fn input(&mut self, command: InputCommand) -> HsrResult<()> {
        if self.cancelled {
            return Err(HsrError::new(
                "HSR-REPLAY-INPUT",
                hints::CANCELLED,
                "scripted input refused because cancellation is active",
            ));
        }
        if !self.foreground {
            return Err(HsrError::new(
                "HSR-REPLAY-INPUT",
                hints::FOCUS_REQUIRED,
                "scripted input refused because foreground ownership was lost",
            ));
        }
        if matches!(command, InputCommand::Click(_)) {
            if self.lose_focus_during_next_click {
                self.lose_focus_during_next_click = false;
                self.foreground = false;
            }
            if self.cancel_during_next_click {
                self.cancel_during_next_click = false;
                self.cancelled = true;
            }
            if self.cancelled {
                return Err(HsrError::new(
                    "HSR-REPLAY-INPUT",
                    hints::CANCELLED,
                    "scripted cancellation occurred before the mouse-button event",
                ));
            }
            if !self.foreground {
                return Err(HsrError::new(
                    "HSR-REPLAY-INPUT",
                    hints::FOCUS_REQUIRED,
                    "scripted focus loss occurred before the mouse-button event",
                ));
            }
            if self.geometry_drifted
                || self.verified_frame_dimensions.is_some_and(|dimensions| {
                    dimensions != (self.identity.width, self.identity.height)
                })
            {
                return Err(HsrError::new(
                    "HSR-REPLAY-GEOMETRY",
                    hints::SCREEN_INVALID,
                    "scripted click refused after client geometry drift",
                ));
            }
        }
        self.commands.push(command);
        if self.lose_focus_after_commands == Some(self.commands.len()) {
            self.foreground = false;
        }
        if self.cancel_after_commands == Some(self.commands.len()) {
            self.cancelled = true;
        }
        if let Some((count, width, height)) = self.geometry_drift_after_commands {
            if count == self.commands.len() {
                self.identity.width = width;
                self.identity.height = height;
                self.geometry_drifted = true;
            }
        }
        Ok(())
    }

    fn wait(&mut self, _duration: Duration) -> HsrResult<()> {
        if self.cancelled {
            Err(HsrError::new(
                "HSR-REPLAY-CANCELLED",
                hints::CANCELLED,
                "scripted wait interrupted by cancellation",
            ))
        } else {
            Ok(())
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yas::cancel::StopReason;

    #[test]
    fn caller_owned_token_can_cancel_before_device_discovery() {
        let cancel = CancelToken::new();
        cancel.cancel(StopReason::UserAbort);

        let error = WindowsHsrDevice::locate_with_cancel(CaptureMethod::Wgc, cancel)
            .err()
            .expect("a pre-cancelled live device must not touch the game window");

        assert_eq!(error.code(), "HSR-DEVICE-CANCELLED");
        assert!(error
            .localized_message(crate::localization::Language::En)
            .contains("safely aborted"));
    }

    #[test]
    fn aspect_validation_accepts_scaled_16_by_9() {
        for (width, height) in [(1280, 720), (1920, 1080), (2560, 1440)] {
            validate_aspect(width, height).unwrap();
        }
        assert_eq!(
            validate_aspect(1600, 1000).unwrap_err().code(),
            "HSR-DEVICE-ASPECT"
        );
    }

    #[test]
    fn replay_refuses_input_after_focus_loss() {
        let mut replay = ReplayDevice::new(1920, 1080, Vec::new());
        replay.set_foreground(false);
        assert!(replay.input(InputCommand::Key('d')).is_err());
        assert!(replay.commands().is_empty());
    }

    #[test]
    fn replay_records_scroll_and_stops_after_scripted_cancel() {
        let mut replay = ReplayDevice::new(1920, 1080, Vec::new());
        replay.cancel_after_commands(1);
        replay.input(InputCommand::Scroll(1)).unwrap();
        assert_eq!(replay.commands(), &[InputCommand::Scroll(1)]);
        let error = replay.input(InputCommand::Scroll(1)).unwrap_err();
        assert_eq!(error.code(), "HSR-REPLAY-INPUT");
        assert!(error
            .localized_message(crate::localization::Language::En)
            .contains("safely aborted"));
        assert_eq!(replay.commands().len(), 1);
    }

    #[test]
    fn geometry_drift_after_a_verified_frame_refuses_the_next_click() {
        let frame = RgbImage::new(1920, 1080);
        let mut replay = ReplayDevice::new(1920, 1080, vec![frame]);
        replay.capture_client().unwrap();
        replay.drift_geometry_after_commands(1, 1600, 900);
        replay.input(InputCommand::Key('b')).unwrap();

        let error = replay
            .input(InputCommand::Click(Point::new(0.5, 0.5)))
            .unwrap_err();
        assert_eq!(error.code(), "HSR-REPLAY-GEOMETRY");
        assert_eq!(replay.commands(), &[InputCommand::Key('b')]);
    }

    #[test]
    fn click_rechecks_focus_and_cancellation_before_button_event() {
        let mut focus = ReplayDevice::new(1920, 1080, Vec::new());
        focus.lose_focus_during_next_click();
        let focus_error = focus
            .input(InputCommand::Click(Point::new(0.5, 0.5)))
            .unwrap_err();
        assert_eq!(focus_error.code(), "HSR-REPLAY-INPUT");
        assert!(focus.commands().is_empty());

        let mut cancelled = ReplayDevice::new(1920, 1080, Vec::new());
        cancelled.cancel_during_next_click();
        let cancel_error = cancelled
            .input(InputCommand::Click(Point::new(0.5, 0.5)))
            .unwrap_err();
        assert_eq!(cancel_error.code(), "HSR-REPLAY-INPUT");
        assert!(cancelled.commands().is_empty());
    }

    #[test]
    fn geometry_validation_rejects_move_resize_and_frame_mismatch() {
        let expected = Rect::new(20, 30, 1920, 1080);
        validate_client_geometry(expected, expected, Some((1920, 1080))).unwrap();
        assert_eq!(
            validate_client_geometry(expected, Rect::new(21, 30, 1920, 1080), Some((1920, 1080)))
                .unwrap_err()
                .code(),
            "HSR-DEVICE-GEOMETRY-DRIFT"
        );
        assert_eq!(
            validate_client_geometry(expected, Rect::new(20, 30, 1600, 900), Some((1920, 1080)))
                .unwrap_err()
                .code(),
            "HSR-DEVICE-GEOMETRY-DRIFT"
        );
    }
}
