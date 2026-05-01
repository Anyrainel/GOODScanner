use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

static HDR_MODE: AtomicBool = AtomicBool::new(false);
static HDR_WHITE_POINT: AtomicU32 = AtomicU32::new(0); // stored as f32 bits; 0 → use default

pub fn set_hdr_mode(enabled: bool) {
    HDR_MODE.store(enabled, Ordering::Relaxed);
}

pub fn is_hdr_mode() -> bool {
    HDR_MODE.load(Ordering::Relaxed)
}

pub fn set_hdr_white_point(wp: f32) {
    HDR_WHITE_POINT.store(wp.to_bits(), Ordering::Relaxed);
}

/// Returns the configured white point, falling back to 4.0 if never set.
pub fn hdr_white_point() -> f32 {
    let bits = HDR_WHITE_POINT.load(Ordering::Relaxed);
    if bits == 0 {
        4.0
    } else {
        f32::from_bits(bits)
    }
}
