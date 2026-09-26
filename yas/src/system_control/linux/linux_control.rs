use enigo::{Enigo, Key, KeyboardControllable, MouseButton, MouseControllable};

pub struct LinuxControl {
    enigo: Enigo,
}

impl LinuxControl {
    pub fn new() -> LinuxControl {
        LinuxControl {
            enigo: Enigo::new(),
        }
    }

    pub fn mouse_move_to(&mut self, x: i32, y: i32) -> anyhow::Result<()> {
        self.enigo.mouse_move_to(x, y);

        anyhow::Ok(())
    }

    pub fn mouse_click(&mut self) -> anyhow::Result<()> {
        self.enigo.mouse_click(MouseButton::Left);

        anyhow::Ok(())
    }

    pub fn mouse_scroll(&mut self, amount: i32, _try_find: bool) -> anyhow::Result<()> {
        self.enigo.mouse_scroll_y(amount);

        anyhow::Ok(())
    }

    pub fn mouse_scroll_wheel_delta(&mut self, delta: i32) -> anyhow::Result<()> {
        let notches = if delta.abs() >= 120 {
            delta / 120
        } else if delta == 0 {
            0
        } else {
            delta.signum()
        };
        if notches != 0 {
            self.mouse_scroll(notches, false)?;
        }
        anyhow::Ok(())
    }

    pub fn key_press(&mut self, key: Key) -> anyhow::Result<()> {
        self.enigo.key_click(key);
        anyhow::Ok(())
    }
}
