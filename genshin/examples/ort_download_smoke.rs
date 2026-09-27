fn main() {
    match genshin_scanner::cli::download_onnxruntime() {
        Ok(()) => println!("SMOKE-OK"),
        Err(e) => println!("SMOKE-FAIL: {e:#}"),
    }
}
