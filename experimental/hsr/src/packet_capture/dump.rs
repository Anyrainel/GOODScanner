//! Explicitly enabled decrypted protobuf diagnostics, isolated from GI dumps.
use crate::{HsrError, HsrResult};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

pub(super) struct PacketDump {
    root: PathBuf,
    session: Option<PathBuf>,
    sequence: u64,
}
impl PacketDump {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            session: None,
            sequence: 0,
        }
    }
    pub fn write(&mut self, bytes: &[u8]) -> HsrResult<()> {
        let result = (|| -> std::io::Result<()> {
            if self.session.is_none() {
                fs::create_dir_all(&self.root)?;
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(std::io::Error::other)?
                    .as_nanos();
                let session = self.root.join(format!("hsr_{stamp}"));
                fs::create_dir(&session)?;
                self.session = Some(session);
            }
            self.sequence += 1;
            let path = self
                .session
                .as_ref()
                .unwrap()
                .join(format!("hsr_{:06}.bin", self.sequence));
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            file.write_all(bytes)?;
            file.flush()
        })();
        result.map_err(|e| {
            HsrError::write_failed(
                "HSR-PACKET-DUMP",
                format!("folder={}; cause={e}", self.root.display()),
            )
        })
    }
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
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unwritable_dump_is_a_reported_error() {
        let mut dump = PacketDump::new(PathBuf::from("NUL/invalid"));
        assert!(dump.write(&[1]).is_err());
    }
}
