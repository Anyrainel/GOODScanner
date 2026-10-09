use std::collections::HashMap;

use rand_mt::Mt64;
use tracing::{debug, info, instrument, trace, warn};

use crate::bytes_as_hex;
use crate::cs_rand::Random;

#[instrument(skip_all)]
pub fn decrypt_command(key: &[u8], encrypted: &mut [u8]) {
    trace!(data = bytes_as_hex(encrypted), "before decryption");

    for i in 0..encrypted.len() {
        encrypted[i] ^= key[i % key.len()];
    }

    trace!(data = bytes_as_hex(encrypted), "after decryption");
}

pub fn lookup_initial_key(initial_keys: &HashMap<u16, Vec<u8>>, bytes: &[u8]) -> Option<Vec<u8>> {
    let version = u16::from_be_bytes(bytes[..2].try_into().unwrap()) ^ 0x4567;

    // attempt to fetch from user provided initial keys, otherwise use our own baked-in ones
    let key = initial_keys.get(&version).cloned();
    match key {
        Some(key) => {
            info!(version, "found initial decryption key");
            Some(key)
        },
        None => {
            info!(version, "didn't find decryption key");
            None
        },
    }
}

pub fn new_key_from_seed(seed: u64) -> Vec<u8> {
    // mersenne twister generator
    let mut first = Mt64::new(seed);
    let mut generator = Mt64::new(first.next_u64());

    let _ = generator.next_u64(); // skip first number

    let mut key = Vec::with_capacity(512);
    for _ in 0..512 {
        for b in generator.next_u64().to_be_bytes() {
            key.push(b);
        }
    }
    key
}

pub fn guess(seed: i64, server_seed: u64, depth: i32, data: Vec<u8>) -> Option<Vec<u8>> {
    // Attempt to generate the key.
    let mut generator = Random::seeded(seed as i32);
    for _ in 0..depth {
        let client_seed = generator.next_safe_uint64();

        let seed = client_seed ^ server_seed;
        let key = new_key_from_seed(seed);

        let mut clone = data.clone();
        decrypt_command(&key, &mut clone);

        if clone[0] == 0x45
            && clone[1] == 0x67
            && clone[clone.len() - 2] == 0x89
            && clone[clone.len() - 1] == 0xAB
        {
            debug!("Found encryption key seed: {seed}");
            return Some(key);
        }
    }

    None
}

pub fn bruteforce(sent_time: u64, server_seed: u64, data: Vec<u8>) -> Option<(u64, Vec<u8>)> {
    debug!("Running bruteforce loop.");
    // Generate new seeds.
    for i in 0..3000i64 {
        let offset = if i % 2 == 0 { i / 2 } else { -(i - 1) / 2 };
        let time = sent_time as i64 + offset; // This will act as the seed.

        if let Some(key) = guess(time, server_seed, 5, data.clone()) {
            return Some((time as u64, key));
        }
    }
    warn!("Unable to find the encryption key seed.");
    None
}
