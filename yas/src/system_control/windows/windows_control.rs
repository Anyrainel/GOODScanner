use enigo::{Enigo, Key, KeyboardControllable, MouseButton, MouseControllable};

pub struct WindowsSystemControl {
    enigo: Enigo,
}

impl WindowsSystemControl {
    pub fn new() -> WindowsSystemControl {
        WindowsSystemControl {
            enigo: Enigo::new(),
        }
    }

    pub fn mouse_move_to(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        self.enigo.mouse_move_to(x, y);

        anyhow::Ok(())
    }

    pub fn mouse_click(&mut self) -> anyhow::Result<()> {
        // Use explicit down/up with a hold delay — enigo's mouse_click sends
        // down+up back-to-back with zero delay. Some game UI elements (especially
        // under high CPU/GPU load or WGC capture) need a minimum hold time to
        // register the click. This matches enigo's key_click which uses 20ms.
        self.enigo.mouse_down(MouseButton::Left);
        std::thread::sleep(std::time::Duration::from_millis(20));
        self.enigo.mouse_up(MouseButton::Left);

        anyhow::Ok(())
    }

    pub fn mouse_scroll(&mut self, amount: i32, _try_find: bool) -> anyhow::Result<()> {
        self.enigo.mouse_scroll_y(amount);

        anyhow::Ok(())
    }

    /// Send a raw `WM_MOUSEWHEEL` delta. 120 is one detent; smaller values
    /// are a high-resolution wheel tick that some apps ignore.
    pub fn mouse_scroll_wheel_delta(&mut self, delta: i32) -> anyhow::Result<()> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_WHEEL, MOUSEINPUT,
        };
        let mut input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: delta as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) };
        if sent != 1 {
            anyhow::bail!("SendInput MOUSEEVENTF_WHEEL failed");
        }
        anyhow::Ok(())
    }

    pub fn key_press(&mut self, key: Key) -> anyhow::Result<()> {
        self.enigo.key_click(key);
        anyhow::Ok(())
    }
}
