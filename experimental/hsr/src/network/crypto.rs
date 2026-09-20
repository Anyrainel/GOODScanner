// Adapted from IceDynamix/reliquary d5cf3b7 (MIT); see THIRD_PARTY_NOTICES.md.
use std::collections::HashMap;

use rand_mt::Mt64;
use tracing::{debug, instrument};

#[instrument(skip_all)]
pub fn decrypt_command(key: &[u8], encrypted: &mut [u8]) {
    for i in 0..encrypted.len() {
        encrypted[i] ^= key[i % key.len()];
    }
}

pub fn get_game_version(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap()) ^ 0x9D74C714
}

pub fn lookup_initial_key(initial_keys: &HashMap<u32, Vec<u8>>, version: u32) -> Option<Vec<u8>> {
    debug!(version = version, "detected game version");

    // attempt to fetch from user provided initial keys, otherwise use our own baked-in ones
    initial_keys.get(&version).cloned()
}

pub fn new_key_from_seed(seed: u64) -> Vec<u8> {
    // mersenne twister generator
    let mut generator = Mt64::new(seed);

    let mut key = Vec::with_capacity(512);
    for _ in 0..512 {
        for b in generator.next_u64().to_be_bytes() {
            key.push(b);
        }
    }
    key
}
