use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Save a bounded HTTP request for debugging, independently of recovery journals.
pub fn save_request_log(directory: &Path, endpoint: &str, body: &[u8]) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for sequence in 0..100 {
        let path = directory.join(format!("{endpoint}_{stamp}_{sequence}.json"));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(body)?;
                file.sync_all()?;
                return Ok(path);
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "request log filename collision",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_requests_preserve_bytes_and_never_overwrite_and_io_errors_propagate() {
        let root = std::env::temp_dir().join(format!(
            "ggscanner-requests-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let body = br#"{"instructions":[{"desired":{"lock":true}}]}"#;
        let first = save_request_log(&root, "hsr_manage", body).unwrap();
        let second = save_request_log(&root, "hsr_manage", body).unwrap();
        assert_ne!(first, second);
        assert_eq!(fs::read(first).unwrap(), body);
        assert_eq!(fs::read(&second).unwrap(), body);
        assert!(save_request_log(&second, "hsr_manage", body).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
