use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;
use serde::{Deserialize, Serialize};

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

const SESSION_TTL_SECS: u64 = 60 * 60 * 24 * 7; // 7 days

#[derive(Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub password_hash: String,
    #[serde(default = "default_role")]
    pub role: String,
    pub created_at: u64,
}

fn default_role() -> String {
    "admin".into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub name: String,
    pub key_hash: String,
    pub created_at: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct AuthFile {
    #[serde(default)]
    users: Vec<User>,
    #[serde(default)]
    api_keys: Vec<ApiKey>,
    #[serde(default)]
    session_secret: String,
}

pub struct AuthStore {
    path: PathBuf,
    file: AuthFile,
}

fn hash_secret(raw: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(raw.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

fn verify_secret(raw: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .ok()
        .map(|parsed| Argon2::default().verify_password(raw.as_bytes(), &parsed).is_ok())
        .unwrap_or(false)
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn sign(secret_hex: &str, msg: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(secret_hex.as_bytes());
    h.update(b"|");
    h.update(msg.as_bytes());
    let out = h.finalize();
    out.iter().map(|b| format!("{b:02x}")).collect()
}

impl AuthStore {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let dir = db_path.join(".torapipe");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("auth.json");
        let mut file: AuthFile = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => AuthFile::default(),
            Err(e) => return Err(e.to_string()),
        };
        if file.session_secret.is_empty() {
            file.session_secret = random_hex(32);
        }
        let store = Self { path, file };
        store.save()?;
        Ok(store)
    }

    fn save(&self) -> Result<(), String> {
        let data = serde_json::to_vec_pretty(&self.file).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, &self.path).map_err(|e| e.to_string())
    }

    pub fn has_users(&self) -> bool {
        !self.file.users.is_empty()
    }

    pub fn create_user(&mut self, name: &str, password: &str) -> Result<String, String> {
        if self.file.users.iter().any(|u| u.name == name) {
            return Err("user already exists".into());
        }
        let id = format!("user_{}_{}", now_secs(), self.file.users.len() + 1);
        self.file.users.push(User {
            id: id.clone(),
            name: name.to_string(),
            password_hash: hash_secret(password)?,
            role: "admin".into(),
            created_at: now_secs(),
        });
        self.save()?;
        Ok(id)
    }

    pub fn login(&self, name: &str, password: &str) -> Option<String> {
        let user = self.file.users.iter().find(|u| u.name == name)?;
        if !verify_secret(password, &user.password_hash) {
            return None;
        }
        let expiry = now_secs() + SESSION_TTL_SECS;
        let msg = format!("{}.{}", user.id, expiry);
        let sig = sign(&self.file.session_secret, &msg);
        Some(format!("{msg}.{sig}"))
    }

    pub fn validate_session(&self, token: &str) -> Option<String> {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let (uid, expiry_s, sig) = (parts[0], parts[1], parts[2]);
        let expiry: u64 = expiry_s.parse().ok()?;
        if now_secs() > expiry {
            return None;
        }
        let expected = sign(&self.file.session_secret, &format!("{uid}.{expiry_s}"));
        if !constant_time_eq(sig, &expected) {
            return None;
        }
        self.file.users.iter().find(|u| u.id == uid).map(|u| u.id.clone())
    }

    pub fn create_api_key(&mut self, name: &str) -> Result<String, String> {
        let raw = format!("tk_{}", random_hex(24));
        let id = format!("key_{}_{}", now_secs(), self.file.api_keys.len() + 1);
        self.file.api_keys.push(ApiKey {
            id,
            name: name.to_string(),
            key_hash: hash_secret(&raw)?,
            created_at: now_secs(),
        });
        self.save()?;
        Ok(raw)
    }

    pub fn verify_api_key(&self, raw: &str) -> bool {
        self.file.api_keys.iter().any(|k| verify_secret(raw, &k.key_hash))
    }

    pub fn user_name(&self, id: &str) -> Option<String> {
        self.file.users.iter().find(|u| u.id == id).map(|u| u.name.clone())
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_and_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AuthStore::open(dir.path()).unwrap();
        assert!(!store.has_users());
        store.create_user("admin", "hunter2").unwrap();
        assert!(store.has_users());
        let token = store.login("admin", "hunter2").expect("login ok");
        assert_eq!(store.validate_session(&token).as_deref().is_some(), true);
        assert!(store.login("admin", "wrong").is_none());
        // tampered token rejected
        assert!(store.validate_session("a.b.c").is_none());
    }

    #[test]
    fn api_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = AuthStore::open(dir.path()).unwrap();
        let raw = store.create_api_key("ci").unwrap();
        assert!(store.verify_api_key(&raw));
        assert!(!store.verify_api_key("tk_bogus"));
    }
}
