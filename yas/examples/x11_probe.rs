//! X11 environment diagnostic: lists titled windows, shows the XTEST
//! extension status, and optionally captures a region of a window to a PNG.
//!
//! Usage:
//!   cargo run -p yas_core --example x11_probe                # list windows
//!   cargo run -p yas_core --example x11_probe -- capture 0x4c00007 out.png
//!                                                           # capture window region

use yas_core::positioning::Rect;
use yas_core::utils;

fn main() {
    println!("DISPLAY = {:?}", std::env::var("DISPLAY").ok());
    println!("WAYLAND_DISPLAY = {:?}", std::env::var("WAYLAND_DISPLAY").ok());
    println!(
        "XTEST available: {}",
        if utils::xtest_available() { "yes" } else { "no" }
    );

    match utils::query_pointer() {
        Ok(p) => println!("pointer: ({}, {}), mask={:#x}", p.root_x, p.root_y, p.mask),
        Err(e) => println!("pointer query failed: {e}"),
    }

    let windows = match utils::enumerate_windows() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("enumerate_windows failed: {e:#}");
            std::process::exit(1);
        },
    };
    println!("\n{} titled window(s):", windows.len());
    for w in &windows {
        let geom = utils::get_client_rect(w.window)
            .map(|r| format!("{r}"))
            .unwrap_or_else(|e| format!("<error {e}>"));
        println!(
            "  {:#010x}  {:>30}  class={:<24}  {}",
            w.window, w.title, w.class, geom
        );
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 4 && args[1] == "capture" {
        let window = u32::from_str_radix(args[2].trim_start_matches("0x"), 16)
            .expect("window id (hex)");
        let out = &args[3];
        let rect = utils::get_client_rect(window).expect("client rect");
        // Capture the top-left quarter as a sanity region.
        let region = Rect::new(
            rect.left,
            rect.top,
            (rect.width / 2).max(1),
            (rect.height / 2).max(1),
        );
        let img = utils::capture_window_region(window, region).expect("capture");
        img.save(out).expect("save png");
        println!("saved {out} ({}x{})", img.width(), img.height());
    }
}
