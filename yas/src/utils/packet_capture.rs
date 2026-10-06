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
            "无法开始抓包：Windows 自带的抓包功能无法加载。\n请先在系统设置中安装 Windows 更新，然后重启电脑。\n若仍失败：在开始菜单搜索“命令提示符”，右键选择“以管理员身份运行”，依次运行 DISM.exe /Online /Cleanup-Image /RestoreHealth 和 sfc /scannow，完成后重启电脑。\n暂时也可以使用“扫描”功能，通过识别游戏画面导出数据。\n无法加载的系统组件：PktMonApi.dll 或其依赖。",
            "Capture could not start: Windows could not load its built-in packet capture feature.\nInstall Windows updates in Settings, then restart your computer.\nIf this still fails: search for Command Prompt in the Start menu, right-click it and choose Run as administrator. Run DISM.exe /Online /Cleanup-Image /RestoreHealth, then sfc /scannow, and restart your computer.\nYou can also use the Scanner to export data by reading the game screen.\nSystem component that could not be loaded: PktMonApi.dll or one of its dependencies.",
        )),
        127 => Some((
            "无法开始抓包：当前 Windows 自带的抓包功能缺少程序需要的功能。请在系统设置中安装 Windows 更新，然后重启电脑。暂时也可以使用“扫描”功能，通过识别游戏画面导出数据。\n系统组件 PktMonApi.dll 缺少所需接口。",
            "Capture could not start: the built-in Windows capture feature is missing a required function. Install Windows updates in Settings, then restart your computer. You can also use the Scanner to export data by reading the game screen.\nPktMonApi.dll is missing a required API function.",
        )),
        5 => Some((
            "无法开始抓包：Windows 没有允许程序使用抓包功能。请关闭 GOODCapture，右键点击程序，选择“以管理员身份运行”，然后重试。",
            "Capture could not start: Windows denied permission to use packet capture. Close GOODCapture, right-click the program, choose Run as administrator, and try again.",
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
