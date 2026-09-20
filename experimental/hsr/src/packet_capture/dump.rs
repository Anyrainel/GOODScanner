//! Explicitly enabled packet diagnostics, isolated from GI dumps.
//!
//! A dump session is created as soon as capture starts so a run with no
//! decrypted commands still proves the option is active. Raw pktmon frames and
//! decrypted protobuf bodies are written as separate numbered files.
use crate::{HsrError, HsrResult};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub(super) struct PacketDump {
    root: PathBuf,
    session: Option<PathBuf>,
    raw_sequence: u64,
    command_sequence: u64,
}
impl PacketDump {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            session: None,
            raw_sequence: 0,
            command_sequence: 0,
        }
    }

    /// Create the session folder before any packet arrives.
    pub fn ensure_session(&mut self) -> HsrResult<PathBuf> {
        self.prepare_session()
            .map_err(|e| dump_error(&self.root, e))?;
        Ok(self.session.clone().expect("session is created"))
    }

    pub fn write_raw(&mut self, bytes: &[u8]) -> HsrResult<()> {
        self.raw_sequence += 1;
        let sequence = self.raw_sequence;
        self.write_file(&format!("raw_{sequence:06}.bin"), bytes)
    }

    pub fn write(&mut self, bytes: &[u8]) -> HsrResult<()> {
        self.command_sequence += 1;
        let sequence = self.command_sequence;
        self.write_file(&format!("hsr_{sequence:06}.bin"), bytes)
    }

    fn write_file(&mut self, name: &str, bytes: &[u8]) -> HsrResult<()> {
        let result = (|| -> std::io::Result<()> {
            self.prepare_session()?;
            let path = self.session.as_ref().unwrap().join(name);
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(bytes)?;
            file.flush()
        })();
        result.map_err(|e| dump_error(&self.root, e))
    }

    fn prepare_session(&mut self) -> std::io::Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        fs::create_dir_all(&self.root)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(std::io::Error::other)?
            .as_nanos();
        let session = self.root.join(format!("hsr_{stamp}"));
        fs::create_dir(&session)?;
        fs::write(
            session.join("dump_enabled.txt"),
            format!(
                "HSR packet dump session\nroot={}\nsession={}\nraw=raw_NNNNNN.bin pktmon frames\ncmd=hsr_NNNNNN.bin decrypted protobuf bodies\n",
                self.root.display(),
                session.display()
            ),
        )?;
        self.session = Some(session);
        Ok(())
    }
}

fn dump_error(root: &Path, error: std::io::Error) -> HsrError {
    HsrError::write_failed(
        "HSR-PACKET-DUMP",
        format!("folder={}; cause={error}", root.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lazy_unique_sessions_preserve_exact_decrypted_payloads() {
        let root = std::env::temp_dir().join(format!(
            "hsr-dump-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut first = PacketDump::new(root.clone());
        assert!(!root.exists());
        first.write(&[0, 128, 255]).unwrap();
        first.write(&[3, 2, 1]).unwrap();
        first.write_raw(&[9, 8, 7]).unwrap();
        let mut second = PacketDump::new(root.clone());
        second.write(&[42]).unwrap();
        assert_ne!(first.session, second.session);
        assert_eq!(
            fs::read(first.session.as_ref().unwrap().join("hsr_000001.bin")).unwrap(),
            [0, 128, 255]
        );
        assert_eq!(
            fs::read(first.session.as_ref().unwrap().join("hsr_000002.bin")).unwrap(),
            [3, 2, 1]
        );
        assert_eq!(
            fs::read(first.session.as_ref().unwrap().join("raw_000001.bin")).unwrap(),
            [9, 8, 7]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ensure_session_creates_marker_before_any_packet() {
        let root = std::env::temp_dir().join(format!(
            "hsr-dump-ready-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut dump = PacketDump::new(root.clone());
        let session = dump.ensure_session().unwrap();
        let marker = fs::read_to_string(session.join("dump_enabled.txt")).unwrap();
        assert!(marker.contains("HSR packet dump session"));
        assert!(session.join("hsr_000001.bin").exists() == false);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unwritable_dump_is_a_reported_error() {
        let mut dump = PacketDump::new(PathBuf::from("NUL/invalid"));
        assert!(dump.write(&[1]).is_err());
        assert!(dump.write_raw(&[1]).is_err());
        assert!(dump.ensure_session().is_err());
    }
}
