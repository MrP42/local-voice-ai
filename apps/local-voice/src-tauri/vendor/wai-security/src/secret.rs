use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Mutex;
use zeroize::Zeroizing;

/// Grenze des Windows Credential Managers; darueber Envelope.
pub const DIRECT_LIMIT: usize = 2500;
const SERVICE: &str = "wai-security";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    NotFound,
    Backend(String),
    Crypto,
}

/// Handle statt Rohwert: traegt nur eine zufaellige ID.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SecretHandle(String);
impl SecretHandle {
    pub fn id(&self) -> &str {
        &self.0
    }
    pub fn from_id(id: &str) -> Self {
        SecretHandle(id.to_string())
    }
}
impl fmt::Debug for SecretHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretHandle({})", self.0)
    }
}

/// Geheimer Wert; kein Display, Debug maskiert, Speicher wird genullt.
pub struct SecretValue(Zeroizing<Vec<u8>>);
impl SecretValue {
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}
impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}

/// Schluessel-Wert-Backend (Keyring oder Fake).
pub trait KeyBackend: Send + Sync {
    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError>;
    fn get(&self, key: &str) -> Result<Vec<u8>, SecretError>;
    fn delete(&self, key: &str) -> Result<(), SecretError>;
}

/// Fake-Backend fuer Tests (nur Speicher).
#[derive(Default)]
pub struct MemoryBackend(Mutex<HashMap<String, Vec<u8>>>);
impl KeyBackend for MemoryBackend {
    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(key.into(), value.to_vec());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Vec<u8>, SecretError> {
        self.0
            .lock()
            .unwrap()
            .get(key)
            .cloned()
            .ok_or(SecretError::NotFound)
    }
    fn delete(&self, key: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap()
            .remove(key)
            .map(|_| ())
            .ok_or(SecretError::NotFound)
    }
}

/// Echtes OS-Backend ueber das keyring-Crate (Credential Manager / Keychain).
pub struct KeyringBackend;
fn map_err(e: keyring::Error) -> SecretError {
    match e {
        keyring::Error::NoEntry => SecretError::NotFound,
        other => SecretError::Backend(other.to_string()),
    }
}
impl KeyBackend for KeyringBackend {
    fn set(&self, key: &str, value: &[u8]) -> Result<(), SecretError> {
        keyring::Entry::new(SERVICE, key)
            .map_err(map_err)?
            .set_secret(value)
            .map_err(map_err)
    }
    fn get(&self, key: &str) -> Result<Vec<u8>, SecretError> {
        keyring::Entry::new(SERVICE, key)
            .map_err(map_err)?
            .get_secret()
            .map_err(map_err)
    }
    fn delete(&self, key: &str) -> Result<(), SecretError> {
        keyring::Entry::new(SERVICE, key)
            .map_err(map_err)?
            .delete_credential()
            .map_err(map_err)
    }
}

/// Ablage fuer Envelope-Chiffretexte.
pub trait BlobStore: Send + Sync {
    fn put(&self, name: &str, data: &[u8]) -> Result<(), SecretError>;
    fn get(&self, name: &str) -> Result<Vec<u8>, SecretError>;
    fn delete(&self, name: &str) -> Result<(), SecretError>;
}

#[derive(Default)]
pub struct MemoryBlobs(Mutex<HashMap<String, Vec<u8>>>);
impl BlobStore for MemoryBlobs {
    fn put(&self, n: &str, d: &[u8]) -> Result<(), SecretError> {
        self.0.lock().unwrap().insert(n.into(), d.to_vec());
        Ok(())
    }
    fn get(&self, n: &str) -> Result<Vec<u8>, SecretError> {
        self.0
            .lock()
            .unwrap()
            .get(n)
            .cloned()
            .ok_or(SecretError::NotFound)
    }
    fn delete(&self, n: &str) -> Result<(), SecretError> {
        self.0
            .lock()
            .unwrap()
            .remove(n)
            .map(|_| ())
            .ok_or(SecretError::NotFound)
    }
}

pub struct FileBlobs(pub PathBuf);
impl FileBlobs {
    fn path(&self, n: &str) -> Result<PathBuf, SecretError> {
        if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(SecretError::Backend("invalid blob name".into()));
        }
        Ok(self.0.join(format!("{n}.blob")))
    }
}
impl BlobStore for FileBlobs {
    fn put(&self, n: &str, d: &[u8]) -> Result<(), SecretError> {
        std::fs::create_dir_all(&self.0).map_err(|e| SecretError::Backend(e.kind().to_string()))?;
        std::fs::write(self.path(n)?, d).map_err(|e| SecretError::Backend(e.kind().to_string()))
    }
    fn get(&self, n: &str) -> Result<Vec<u8>, SecretError> {
        std::fs::read(self.path(n)?).map_err(|_| SecretError::NotFound)
    }
    fn delete(&self, n: &str) -> Result<(), SecretError> {
        std::fs::remove_file(self.path(n)?).map_err(|_| SecretError::NotFound)
    }
}

pub trait SecretStore {
    fn put(&self, value: &[u8]) -> Result<SecretHandle, SecretError>;
    fn get(&self, handle: &SecretHandle) -> Result<SecretValue, SecretError>;
    fn delete(&self, handle: &SecretHandle) -> Result<(), SecretError>;
}

/// Kleine Werte direkt im Keyring (Tag 0), grosse als Envelope (Tag 1):
/// Datenschluessel + Key-ID im Keyring, Inhalt AEAD-verschluesselt in der Blob-Ablage.
/// Kein Klartext-Fallback: Fehler des Backends werden durchgereicht.
pub struct EnvelopeStore<K: KeyBackend, B: BlobStore> {
    keys: K,
    blobs: B,
}

impl<K: KeyBackend, B: BlobStore> EnvelopeStore<K, B> {
    pub fn new(keys: K, blobs: B) -> Self {
        Self { keys, blobs }
    }
}

fn rand<const N: usize>() -> Result<[u8; N], SecretError> {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).map_err(|_| SecretError::Crypto)?;
    Ok(b)
}
fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl<K: KeyBackend, B: BlobStore> SecretStore for EnvelopeStore<K, B> {
    fn put(&self, value: &[u8]) -> Result<SecretHandle, SecretError> {
        let id = hex(&rand::<16>()?);
        if value.len() <= DIRECT_LIMIT {
            let mut v = Zeroizing::new(Vec::with_capacity(value.len() + 1));
            v.push(0);
            v.extend_from_slice(value);
            self.keys.set(&id, &v)?;
        } else {
            let dek = Zeroizing::new(rand::<32>()?);
            let key_id = rand::<16>()?;
            let nonce = rand::<12>()?;
            let aad = [id.as_bytes(), &key_id].concat();
            let ct = ChaCha20Poly1305::new(Key::from_slice(&*dek))
                .encrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: value,
                        aad: &aad,
                    },
                )
                .map_err(|_| SecretError::Crypto)?;
            let mut meta = Zeroizing::new(vec![1u8]);
            meta.extend_from_slice(&key_id);
            meta.extend_from_slice(&*dek);
            self.blobs
                .put(&id, &[&key_id[..], &nonce[..], &ct[..]].concat())?;
            if let Err(e) = self.keys.set(&id, &meta) {
                let _ = self.blobs.delete(&id);
                return Err(e);
            }
        }
        Ok(SecretHandle(id))
    }

    fn get(&self, h: &SecretHandle) -> Result<SecretValue, SecretError> {
        let raw = Zeroizing::new(self.keys.get(&h.0)?);
        match raw.first() {
            Some(0) => Ok(SecretValue(Zeroizing::new(raw[1..].to_vec()))),
            Some(1) if raw.len() == 49 => {
                let (key_id, dek) = (&raw[1..17], &raw[17..49]);
                let blob = self.blobs.get(&h.0)?;
                if blob.len() < 28 || &blob[..16] != key_id {
                    return Err(SecretError::Crypto);
                }
                let aad = [h.0.as_bytes(), key_id].concat();
                let pt = ChaCha20Poly1305::new(Key::from_slice(dek))
                    .decrypt(
                        Nonce::from_slice(&blob[16..28]),
                        Payload {
                            msg: &blob[28..],
                            aad: &aad,
                        },
                    )
                    .map_err(|_| SecretError::Crypto)?;
                Ok(SecretValue(Zeroizing::new(pt)))
            }
            _ => Err(SecretError::Crypto),
        }
    }

    fn delete(&self, h: &SecretHandle) -> Result<(), SecretError> {
        let raw = Zeroizing::new(self.keys.get(&h.0)?);
        self.keys.delete(&h.0)?;
        if raw.first() == Some(&1) {
            let _ = self.blobs.delete(&h.0);
        }
        Ok(())
    }
}
