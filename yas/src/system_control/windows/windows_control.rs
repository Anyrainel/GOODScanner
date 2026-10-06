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

    pub fn mouse_drag(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) -> anyhow::Result<()> {
        self.enigo.mouse_move_to(x0, y0);
        std::thread::sleep(std::time::Duration::from_millis(40));
        self.enigo.mouse_down(MouseButton::Left);
        std::thread::sleep(std::time::Duration::from_millis(40));
        let steps = 16;
        for step in 1..=steps {
            let x = x0 + (x1 - x0) * step / steps;
            let y = y0 + (y1 - y0) * step / steps;
            self.enigo.mouse_move_to(x, y);
            std::thread::sleep(std::time::Duration::from_millis(35));
        }
        // Stop while still holding so a precise page drag does not become a
        // fling with inertia after the button is released.
        std::thread::sleep(std::time::Duration::from_millis(180));
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
