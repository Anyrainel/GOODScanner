use yas::positioning::{Pos, Rect};

/// Translates 1920x1080 base coordinates to any 16:9 game resolution.
///
/// The GOODScanner defines all coordinates at 1920x1080 base resolution.
/// This scaler computes the ratio from the actual game window size and
/// provides methods to translate positions, rectangles, and individual values.
#[derive(Debug, Clone)]
pub struct CoordScaler {
    scale_x: f64,
    scale_y: f64,
}

/// Base resolution width used by all GOODScanner coordinates
pub const BASE_WIDTH: f64 = 1920.0;
/// Base resolution height used by all GOODScanner coordinates
pub const BASE_HEIGHT: f64 = 1080.0;

impl CoordScaler {
    /// Create a new scaler from the actual game window dimensions.
    ///
    /// For 16:9 aspect ratios, `scale_x` and `scale_y` will be equal.
    /// Non-16:9 ratios are supported but may produce stretched coordinates.
    pub fn new(game_width: u32, game_height: u32) -> Self {
        Self {
            scale_x: game_width as f64 / BASE_WIDTH,
            scale_y: game_height as f64 / BASE_HEIGHT,
        }
    }

    /// Scale an X coordinate from 1920-base to actual resolution
    pub fn scale_x(&self, x: f64) -> f64 {
        x * self.scale_x
    }

    /// Scale a Y coordinate from 1080-base to actual resolution
    pub fn scale_y(&self, y: f64) -> f64 {
        y * self.scale_y
    }

    /// Scale a Pos from base resolution to actual resolution
    pub fn scale_pos(&self, pos: &Pos<f64>) -> Pos<f64> {
        Pos {
            x: pos.x * self.scale_x,
            y: pos.y * self.scale_y,
        }
    }

    /// Scale a Pos and convert to integer
    pub fn scale_pos_i32(&self, pos: &Pos<f64>) -> Pos<i32> {
        Pos {
            x: (pos.x * self.scale_x) as i32,
            y: (pos.y * self.scale_y) as i32,
        }
    }

    /// Scale a Rect from base resolution to actual resolution
    pub fn scale_rect(&self, rect: &Rect<f64>) -> Rect<f64> {
        Rect {
            left: rect.left * self.scale_x,
            top: rect.top * self.scale_y,
            width: rect.width * self.scale_x,
            height: rect.height * self.scale_y,
        }
    }

    /// Scale a Rect and convert to integer
    pub fn scale_rect_i32(&self, rect: &Rect<f64>) -> Rect<i32> {
        Rect {
            left: (rect.left * self.scale_x) as i32,
            top: (rect.top * self.scale_y) as i32,
            width: (rect.width * self.scale_x) as i32,
            height: (rect.height * self.scale_y) as i32,
        }
    }

    /// Create a Rect from base-resolution coordinates (x, y, w, h) and scale it
    pub fn rect(&self, x: f64, y: f64, w: f64, h: f64) -> Rect<i32> {
        Rect {
            left: (x * self.scale_x) as i32,
            top: (y * self.scale_y) as i32,
            width: (w * self.scale_x) as i32,
            height: (h * self.scale_y) as i32,
        }
    }

    /// Create a Pos from base-resolution coordinates (x, y) and scale it
    pub fn pos(&self, x: f64, y: f64) -> Pos<i32> {
        Pos {
            x: (x * self.scale_x) as i32,
            y: (y * self.scale_y) as i32,
        }
    }

    /// Scale a single value using the X scale factor
    pub fn x(&self, val: f64) -> i32 {
        (val * self.scale_x) as i32
    }

    /// Scale a single value using the Y scale factor
    pub fn y(&self, val: f64) -> i32 {
        (val * self.scale_y) as i32
    }

    /// Get the X scale factor
    pub fn factor_x(&self) -> f64 {
        self.scale_x
    }

    /// Get the Y scale factor
    pub fn factor_y(&self) -> f64 {
        self.scale_y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_scale() {
        let s = CoordScaler::new(1920, 1080);
        assert_eq!(s.x(100.0), 100);
        assert_eq!(s.y(200.0), 200);
    }

    #[test]
    fn test_2x_scale() {
        let s = CoordScaler::new(3840, 2160);
        assert_eq!(s.x(100.0), 200);
        assert_eq!(s.y(100.0), 200);
    }

    #[test]
    fn test_common_resolution() {
        let s = CoordScaler::new(2560, 1440);
        // 2560/1920 = 1.333...
        assert_eq!(s.x(1920.0), 2560);
        assert_eq!(s.y(1080.0), 1440);
    }
}
