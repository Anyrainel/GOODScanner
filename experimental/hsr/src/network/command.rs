use std::fmt;
use thiserror::Error;
use tracing::{instrument, warn};

/// Game command header.
///
/// Contains the type of the command in `command_id`
/// and the data encoded in protobuf in `proto_data`
///
/// ## Bit Layout
/// | Bit indices     |  Type |  Name |
/// | - | - | - |
/// |   0..4      |  `u32`  |  Header (magic constant) |
/// |   0..6      |  `u16`  |  command_id |
/// |   6..8      |  `u16`  |  header_len (unsure) |
/// |   8..12     |  `u32`  |  data_len |
/// |  12..12+data_len |  variable  |  proto_data |
/// | data_len..data_len+4  |  `u32`  |  Tail (magic constant) |
#[derive(Clone)]
pub struct GameCommand {
    pub command_id: u16,
    #[allow(unused)]
    pub header_len: u16,
    #[allow(unused)]
    pub data_len: u32,
    #[allow(unused)]
    pub proto_header: Vec<u8>,
    pub proto_data: Vec<u8>,
}

impl GameCommand {
    const HEADER_LEN: usize = 12;
    const TAIL_LEN: usize = 4;

    #[instrument(skip(bytes), fields(len = bytes.len()))]
    pub fn try_new(bytes: Vec<u8>) -> Result<Self, GameCommandError> {
        let header_overhead = Self::HEADER_LEN + Self::TAIL_LEN;
        if bytes.len() < header_overhead {
            warn!(len = bytes.len(), "game command header incomplete");
            return Err(GameCommandError::HeaderTooShort {
                expected: header_overhead,
                actual: bytes.len(),
            });
        }

        // skip header magic const
        let command_id = u16::from_be_bytes(bytes[4..6].try_into().unwrap());
        let header_len = u16::from_be_bytes(bytes[6..8].try_into().unwrap());
        let data_len = u32::from_be_bytes(bytes[8..12].try_into().unwrap());

        let data_start = 12 + header_len as usize;
        let data_end = data_start + data_len as usize;

        if data_end.checked_add(4) != Some(bytes.len()) {
            warn!(len = bytes.len(), "game command buffer too short");
            return Err(GameCommandError::CommandTooShort {
                expected: data_end,
                actual: bytes.len(),
            });
        }

        let proto_header = bytes[12..data_start].to_vec();
        let proto_data = bytes[data_start..data_end].to_vec();

        Ok(GameCommand {
            command_id,
            header_len,
            data_len,
            proto_header,
            proto_data,
        })
    }
}

impl fmt::Debug for GameCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameCommand")
            .field("command_id", &self.command_id)
            .field("header_len", &self.header_len)
            .field("data_len", &self.data_len)
            .finish()
    }
}

#[derive(Error, Debug)]
pub enum GameCommandError {
    #[error("command header must be at least {expected} bytes, but was {actual}")]
    HeaderTooShort { expected: usize, actual: usize },
    #[error("command buffer must be at least {expected} bytes, but was {actual}")]
    CommandTooShort { expected: usize, actual: usize },
    #[error("decryption key is missing for command")]
    DecryptionKeyMissing,
    #[error("decrypted version does not match expected version")]
    VersionMismatch,
}
