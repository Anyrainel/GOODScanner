//! Diagnostics shared by the games using the Windows pktmon backend.

use std::io;

/// Match numeric errors, since Windows supplies localized error text and the
/// pktmon dependency can return a Win32 HRESULT inside `io::Error`.
pub fn initialization_hint(error: &io::Error) -> Option<(&'static str, &'static str)> {
    let raw = error.raw_os_error()? as u32;
    let code = if raw & 0xffff_0000 == 0x8007_0000 {
        raw & 0xffff
    } else {
        raw
    };
    match code {
        126 => Some((
            "无法加载 Windows 抓包组件 PktMonApi.dll 或其依赖。请先更新 Windows 并重启；若仍失败，请在管理员命令提示符中依次运行 DISM.exe /Online /Cleanup-Image /RestoreHealth 和 sfc /scannow，完成后重启。也可改用 OCR 扫描。",
            "Windows could not load PktMonApi.dll or one of its dependencies. Update Windows and restart. If capture still fails, run DISM.exe /Online /Cleanup-Image /RestoreHealth, then sfc /scannow in an administrator Command Prompt, and restart. You can also use OCR scanning.",
        )),
        127 => Some((
            "Windows 抓包组件 PktMonApi.dll 缺少所需功能。请更新 Windows 并重启，或改用 OCR 扫描。",
            "The Windows PktMonApi.dll capture component is missing a required function. Update Windows and restart, or use OCR scanning.",
        )),
        5 => Some((
            "Windows 拒绝访问抓包组件。请以管理员身份运行 GOODCapture 后重试。",
            "Windows denied access to the packet capture component. Run GOODCapture as administrator and try again.",
        )),
        _ => None,
    }
}

/// Add actionable context while retaining the original Windows error/code.
pub fn initialization_error(error: io::Error) -> anyhow::Error {
    let hint = initialization_hint(&error);
    let error = anyhow::Error::from(error).context("pktmon backend initialization failed");
    match hint {
        Some((zh, en)) => error.context(format!("{zh} / {en}")),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_win32_errors_and_their_signed_hresults() {
        for (code, expected) in [
            (126u32, "PktMonApi.dll"),
            (127, "required function"),
            (5, "administrator"),
        ] {
            for raw in [code, 0x8007_0000 | code] {
                let error = io::Error::from_raw_os_error(raw as i32);
                let (zh, en) = initialization_hint(&error).unwrap();
                assert!(!zh.is_empty());
                assert!(en.contains(expected));
                let diagnostic = initialization_error(error);
                assert_eq!(
                    diagnostic
                        .downcast_ref::<io::Error>()
                        .unwrap()
                        .raw_os_error(),
                    Some(raw as i32)
                );
                assert!(format!("{diagnostic:#}").contains(&format!("os error {}", raw as i32)));
            }
        }
    }

    #[test]
    fn does_not_misclassify_unknown_errors_or_other_hresult_facilities() {
        for raw in [2u32, 50, 0x8004_007e] {
            let error = io::Error::from_raw_os_error(raw as i32);
            assert!(initialization_hint(&error).is_none());
            assert_eq!(
                initialization_error(error).to_string(),
                "pktmon backend initialization failed"
            );
        }
        assert!(initialization_hint(&io::Error::other("custom backend error")).is_none());
    }
}
