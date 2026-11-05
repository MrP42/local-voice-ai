pub mod sync {
    pub mod crypto {
        use sha2::{Digest, Sha256};
        pub const KEY_LEN: usize = 32;
        pub fn key_id(key: &[u8; 32]) -> String {
            let d = Sha256::digest(key);
            d[..4].iter().map(|b| format!("{b:02x}")).collect()
        }
    }
    pub mod blobs {
        pub fn sha256_hex(_: &[u8]) -> String {
            String::new()
        }
    }
    pub mod account {
        pub struct SyncConfig {
            pub url: String,
            pub token: String,
            pub user_id: i64,
            pub user_email: String,
        }
        impl SyncConfig {
            pub fn key(&self) -> Result<[u8; 32], String> {
                Ok(key_now())
            }
        }
        #[derive(Clone)]
        pub struct Cfg {
            pub url: String,
            pub token: String,
            pub user_id: i64,
            pub user_email: String,
            pub key: [u8; 32],
        }
        pub static CFG: std::sync::Mutex<Option<Cfg>> = std::sync::Mutex::new(None);
        thread_local! { static KEY: std::cell::Cell<[u8; 32]> = std::cell::Cell::new([0; 32]); }
        pub fn load(_: &tauri::AppHandle) -> Option<SyncConfig> {
            let c = CFG.lock().unwrap().clone()?;
            KEY.with(|k| k.set(c.key));
            Some(SyncConfig {
                url: c.url,
                token: c.token,
                user_id: c.user_id,
                user_email: c.user_email,
            })
        }
        pub fn key_now() -> [u8; 32] {
            CFG.lock().unwrap().as_ref().unwrap().key
        }
    }
    #[path = "../../../../apps/local-voice/src-tauri/src/sync/client.rs"]
    pub mod client;
    #[path = "../../../../apps/local-voice/src-tauri/src/sync/pairing.rs"]
    pub mod pairing;
    #[path = "../../../../apps/local-voice/src-tauri/src/sync/pairing_proto.rs"]
    pub mod pairing_proto;
}

#[cfg(test)]
mod e2e;
