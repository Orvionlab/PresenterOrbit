use aes::Aes256;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ctr::cipher::{KeyIvInit, StreamCipher};
use serde_json::Value;
use sha2::{Digest, Sha512_256};

type Aes256Ctr64 = ctr::Ctr64BE<Aes256>;

const IV_BYTES: usize = 16;
const MAX_ENCODED_BYTES: usize = 1024 * 1024;
const MAX_DECODED_BYTES: usize = 1024 * 1024;
const MAX_PACKET_BYTES: usize = (MAX_ENCODED_BYTES / 4) * 3;
const MAX_PLAINTEXT_BYTES: usize = MAX_PACKET_BYTES - IV_BYTES;

pub(super) struct Codec {
    key: [u8; 32],
}

impl Codec {
    pub(super) fn new(password: &str) -> Self {
        Self {
            key: Sha512_256::digest(password.as_bytes()).into(),
        }
    }

    pub(super) fn encode(&self, value: &Value) -> Result<String, String> {
        let plaintext = serde_json::to_vec(value)
            .map_err(|error| format!("Failed to serialize remote message: {error}"))?;
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err("Remote message exceeds the 1 MiB encoded limit".to_string());
        }

        let mut iv = [0; IV_BYTES];
        getrandom::fill(&mut iv)
            .map_err(|error| format!("Failed to generate remote message IV: {error}"))?;

        let encoded = STANDARD.encode(self.encrypt_packet(&plaintext, iv)?);
        if encoded.len() > MAX_ENCODED_BYTES {
            return Err("Remote message exceeds the 1 MiB encoded limit".to_string());
        }
        Ok(encoded)
    }

    pub(super) fn decode(&self, encoded: &str) -> Result<Value, String> {
        if encoded.len() > MAX_ENCODED_BYTES {
            return Err("Encoded remote message exceeds the 1 MiB limit".to_string());
        }

        let mut packet = STANDARD.decode(encoded).map_err(|_| {
            "Invalid base64 remote message or decoded message exceeds the limit".to_string()
        })?;
        if packet.len() > MAX_DECODED_BYTES {
            return Err("Decoded remote message exceeds the 1 MiB limit".to_string());
        }
        if packet.len() < IV_BYTES {
            return Err("Encrypted remote message is too short".to_string());
        }

        let mut iv = [0; IV_BYTES];
        iv.copy_from_slice(&packet[..IV_BYTES]);
        self.apply_cipher(&mut packet[IV_BYTES..], &iv)?;
        serde_json::from_slice(&packet[IV_BYTES..])
            .map_err(|error| format!("Invalid encrypted remote JSON: {error}"))
    }

    fn encrypt_packet(&self, plaintext: &[u8], iv: [u8; IV_BYTES]) -> Result<Vec<u8>, String> {
        let mut packet = Vec::with_capacity(IV_BYTES + plaintext.len());
        packet.extend_from_slice(&iv);
        packet.extend_from_slice(plaintext);
        self.apply_cipher(&mut packet[IV_BYTES..], &iv)?;
        Ok(packet)
    }

    fn apply_cipher(&self, bytes: &mut [u8], iv: &[u8; IV_BYTES]) -> Result<(), String> {
        let mut cipher = Aes256Ctr64::new_from_slices(&self.key, iv)
            .map_err(|_| "Invalid remote message cipher key or IV length".to_string())?;
        cipher
            .try_apply_keystream(bytes)
            .map_err(|_| "Remote message cipher counter exhausted".to_string())
    }
}
