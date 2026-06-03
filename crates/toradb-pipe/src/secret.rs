use std::path::Path;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use base64::Engine;
use rand::RngCore;

const PREFIX: &str = "enc:v1:";

#[derive(Clone)]
pub struct SecretBox {
    key: [u8; 32],
}

impl SecretBox {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Ok(env) = std::env::var("TORADB_SECRET_KEY") {
            let key = decode_key(env.trim())
                .ok_or("TORADB_SECRET_KEY must be 32 bytes (base64 or hex)")?;
            return Ok(Self { key });
        }
        let dir = db_path.join(".torapipe");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("keyfile");
        if let Ok(bytes) = std::fs::read(&path) {
            if bytes.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                return Ok(Self { key });
            }
        }
        // Generate a fresh key and persist with 0600 perms.
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        std::fs::write(&path, key).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, String> {
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| format!("encrypt failed: {e}"))?;
        let mut blob = Vec::with_capacity(12 + ct.len());
        blob.extend_from_slice(&nonce_bytes);
        blob.extend_from_slice(&ct);
        Ok(format!(
            "{PREFIX}{}",
            base64::engine::general_purpose::STANDARD.encode(blob)
        ))
    }

    pub fn decrypt(&self, value: &str) -> Result<String, String> {
        let Some(b64) = value.strip_prefix(PREFIX) else {
            return Ok(value.to_string());
        };
        let blob = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| e.to_string())?;
        if blob.len() < 12 {
            return Err("ciphertext too short".into());
        }
        let (nonce_bytes, ct) = blob.split_at(12);
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.key));
        let pt = cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ct)
            .map_err(|e| format!("decrypt failed: {e}"))?;
        String::from_utf8(pt).map_err(|e| e.to_string())
    }
}

fn decode_key(s: &str) -> Option<[u8; 32]> {
    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(s) {
        if bytes.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            return Some(k);
        }
    }
    if s.len() == 64 {
        let mut k = [0u8; 32];
        for i in 0..32 {
            k[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
        }
        return Some(k);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let sb = SecretBox { key: [7u8; 32] };
        let ct = sb.encrypt("postgres://u:pw@host/db").unwrap();
        assert!(ct.starts_with(PREFIX));
        assert_eq!(sb.decrypt(&ct).unwrap(), "postgres://u:pw@host/db");
    }

    #[test]
    fn plaintext_passthrough() {
        let sb = SecretBox { key: [1u8; 32] };
        assert_eq!(sb.decrypt("sqlite:///x.db").unwrap(), "sqlite:///x.db");
    }

    #[test]
    fn wrong_key_fails() {
        let a = SecretBox { key: [1u8; 32] };
        let b = SecretBox { key: [2u8; 32] };
        let ct = a.encrypt("secret").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }
}
