//! Dateiablage des Bibliothek-Syncs (lva#210, Spec
//! `docs/superpowers/specs/2026-11-02-bibliothek-sync.md`, Abschnitt „Dateien“).
//!
//! Eine Datei (Buchtext, Cover, Hörbuchteil) wird in Stücke zu 4 MiB Klartext
//! geteilt; jedes Stück ist einzeln mit XChaCha20-Poly1305 versiegelt
//! (`nonce ‖ chiffrat ‖ tag`, roh). Die Blob-Kennung ist
//! `hex(SHA-256(key ‖ "lv-blob-v1" ‖ SHA-256(klartext)))`: gleiche Datei,
//! gleiche Kennung, und der Server kann sie keinem Inhalt zuordnen.
//! Gegenstück in Swift: `LibrarySyncFormat` (VoiceCore).

use super::client::{CommitOutcome, HubClient, HubError};
use super::crypto::KEY_LEN;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Klartext je Stück.
pub const CHUNK_PLAIN: usize = 4 * 1024 * 1024;
const NONCE_LEN: usize = 24;
const TAG_LEN: usize = 16;
/// Obergrenze des Hubs (`VoiceBlobService::MAX_CHUNKS`).
pub const MAX_CHUNKS: u64 = 4096;

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

pub fn valid_blob_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Inhaltsadresse aus dem SHA-256 des Klartexts (roh, 32 Byte).
pub fn blob_id(key: &[u8; KEY_LEN], plaintext_sha256: &[u8; 32]) -> String {
    let mut h = Sha256::new();
    h.update(key);
    h.update(b"lv-blob-v1");
    h.update(plaintext_sha256);
    hex(&h.finalize())
}

/// Anzahl der Stücke: eine leere Datei ist ein (leeres) Stück.
pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(CHUNK_PLAIN as u64).max(1)
}

pub fn chunk_aad(user_id: i64, blob_id: &str, index: u64) -> Vec<u8> {
    format!("{user_id}\u{1f}blob\u{1f}{blob_id}\u{1f}{index}").into_bytes()
}

pub fn seal_chunk(
    key: &[u8; KEY_LEN],
    user_id: i64,
    blob_id: &str,
    index: u64,
    plain: &[u8],
) -> Result<Vec<u8>, String> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    seal_chunk_with_nonce(key, user_id, blob_id, index, plain, &nonce)
}

pub fn seal_chunk_with_nonce(
    key: &[u8; KEY_LEN],
    user_id: i64,
    blob_id: &str,
    index: u64,
    plain: &[u8],
    nonce: &[u8; NONCE_LEN],
) -> Result<Vec<u8>, String> {
    let aad = chunk_aad(user_id, blob_id, index);
    let ct = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plain,
                aad: &aad,
            },
        )
        .map_err(|_| "Verschlüsselung eines Stücks fehlgeschlagen".to_string())?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn open_chunk(
    key: &[u8; KEY_LEN],
    user_id: i64,
    blob_id: &str,
    index: u64,
    sealed: &[u8],
) -> Result<Vec<u8>, String> {
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err("Stück zu kurz".into());
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    let aad = chunk_aad(user_id, blob_id, index);
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), Payload { msg: ct, aad: &aad })
        .map_err(|_| {
            "Stück lässt sich nicht entschlüsseln (falscher Schlüssel oder manipuliert)".into()
        })
}

// ---------------------------------------------------------------------------
// Prüfsummen lokaler Dateien (Zwischenspeicher im Ledger)
// ---------------------------------------------------------------------------

/// Größe, Änderungszeit und SHA-256 einer Datei. Solange Größe und Zeit
/// gleich bleiben, wird die Datei nicht erneut gelesen.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct FileStamp {
    pub size: u64,
    pub mtime_ms: i64,
    pub sha256: String,
}

fn stat(path: &Path) -> Result<(u64, i64), String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();
    Ok((meta.len(), mtime))
}

/// Größe und Änderungszeit (ms) einer Datei, wie sie [`FileStamp`] führt.
pub fn stat_of(path: &Path) -> Result<(u64, i64), String> {
    stat(path)
}

pub fn file_sha256(path: &Path) -> Result<[u8; 32], String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().into())
}

/// Prüfsumme aus dem Zwischenspeicher oder frisch gelesen. `budget` begrenzt,
/// wie viele Bytes ein Lauf neu lesen darf; `Ok(None)` heißt „später“.
pub fn stamp(
    cache: &mut BTreeMap<String, FileStamp>,
    path: &Path,
    budget: &mut u64,
) -> Result<Option<FileStamp>, String> {
    let (size, mtime_ms) = stat(path)?;
    let k = path.to_string_lossy().into_owned();
    if let Some(s) = cache.get(&k) {
        if s.size == size && s.mtime_ms == mtime_ms {
            return Ok(Some(s.clone()));
        }
    }
    if size > *budget {
        return Ok(None);
    }
    *budget -= size;
    let s = FileStamp {
        size,
        mtime_ms,
        sha256: hex(&file_sha256(path)?),
    };
    cache.insert(k, s.clone());
    Ok(Some(s))
}

pub fn stamp_bytes(bytes: &[u8]) -> FileStamp {
    FileStamp {
        size: bytes.len() as u64,
        mtime_ms: 0,
        sha256: sha256_hex(bytes),
    }
}

/// Blob-Kennung zu einer Prüfsumme (Hex).
pub fn blob_id_of(key: &[u8; KEY_LEN], stamp: &FileStamp) -> Option<String> {
    unhex32(&stamp.sha256).map(|sha| blob_id(key, &sha))
}

// ---------------------------------------------------------------------------
// Hoch- und Herunterladen
// ---------------------------------------------------------------------------

/// Woher der Klartext eines Uploads kommt.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Bytes(Vec<u8>),
    File(PathBuf),
}

impl Source {
    fn read_chunk(&self, index: u64, size: u64) -> Result<Vec<u8>, String> {
        let start = index * CHUNK_PLAIN as u64;
        let len = (size.saturating_sub(start)).min(CHUNK_PLAIN as u64) as usize;
        match self {
            Source::Bytes(b) => Ok(b
                .get(start as usize..start as usize + len)
                .ok_or("Stück außerhalb der Daten")?
                .to_vec()),
            Source::File(p) => {
                let mut f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
                f.seek(SeekFrom::Start(start))
                    .map_err(|e| format!("{}: {e}", p.display()))?;
                let mut buf = vec![0u8; len];
                f.read_exact(&mut buf)
                    .map_err(|e| format!("{} geändert oder gekürzt: {e}", p.display()))?;
                Ok(buf)
            }
        }
    }
}

/// Ergebnis eines Upload-Versuchs.
#[derive(Debug, PartialEq)]
pub enum Upload {
    /// Auf dem Hub abgeschlossen.
    Done,
    /// Budget des Laufs erschöpft oder Stücke fehlen noch: nächster Lauf.
    Later,
}

/// Eine Datei hochladen und abschließen. Was der Hub schon hat, wird
/// übersprungen; so setzt ein abgebrochener Upload fort. `budget` zählt
/// Klartext-Bytes dieses Laufs. `expected` ist der Stand, aus dem die
/// Kennung stammt: ändert sich die Datei unterwegs, wird nicht
/// abgeschlossen (ein abgeschlossener Blob ist auf dem Hub unveränderlich).
#[allow(clippy::too_many_arguments)]
pub async fn upload(
    client: &HubClient,
    key: &[u8; KEY_LEN],
    user_id: i64,
    blob_id: &str,
    source: &Source,
    size: u64,
    expected: Option<&FileStamp>,
    budget: &mut u64,
) -> Result<Upload, HubError> {
    let chunks = chunk_count(size);
    if chunks > MAX_CHUNKS {
        return Err(HubError::Other("Datei zu groß für die Dateiablage".into()));
    }
    let unchanged = || -> Result<(), HubError> {
        if let (Source::File(p), Some(e)) = (source, expected) {
            let (sz, mt) = stat(p).map_err(HubError::Other)?;
            if sz != e.size || mt != e.mtime_ms {
                return Err(HubError::Other(format!(
                    "{} während des Hochladens geändert",
                    p.display()
                )));
            }
        }
        Ok(())
    };
    let have: std::collections::HashSet<i64> = match client.blob_info(blob_id).await? {
        Some(info) if info.complete => return Ok(Upload::Done),
        Some(info) => info.chunks.iter().map(|c| c.idx).collect(),
        None => Default::default(),
    };
    // Wird alles in diesem Lauf gelesen, prüft die Prüfsumme den Inhalt.
    let mut whole = have.is_empty().then(Sha256::new);
    for i in 0..chunks {
        if have.contains(&(i as i64)) {
            continue;
        }
        if *budget == 0 {
            return Ok(Upload::Later);
        }
        unchanged()?;
        let plain = source.read_chunk(i, size).map_err(HubError::Other)?;
        if let Some(h) = whole.as_mut() {
            h.update(&plain);
        }
        let sealed = seal_chunk(key, user_id, blob_id, i, &plain).map_err(HubError::Other)?;
        client.put_chunk(blob_id, i as usize, &sealed).await?;
        *budget = budget.saturating_sub(plain.len().max(1) as u64);
    }
    unchanged()?;
    if let Some(h) = whole {
        let sha: [u8; 32] = h.finalize().into();
        if self::blob_id(key, &sha) != blob_id {
            return Err(HubError::Other(
                "Datei passt nicht mehr zu ihrer Kennung (während des Hochladens geändert)".into(),
            ));
        }
    }
    match client.commit_blob(blob_id, chunks as usize).await? {
        CommitOutcome::Complete => Ok(Upload::Done),
        CommitOutcome::Missing(_) => Ok(Upload::Later),
    }
}

/// Einen Blob herunterladen, jedes Stück prüfen, den ganzen Klartext gegen
/// die Kennung prüfen und erst dann atomar an `target` legen. Liefert den
/// SHA-256 des Klartexts (Hex).
pub async fn download(
    client: &HubClient,
    key: &[u8; KEY_LEN],
    user_id: i64,
    blob_id: &str,
    size: u64,
    target: &Path,
) -> Result<String, HubError> {
    let chunks = chunk_count(size);
    if !valid_blob_id(blob_id) || chunks > MAX_CHUNKS {
        return Err(HubError::Other("Ungültiger Dateiverweis".into()));
    }
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| HubError::Other(format!("{}: {e}", dir.display())))?;
    }
    let tmp = target.with_extension(format!("sync-{}", std::process::id()));
    let result: Result<String, HubError> = async {
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| HubError::Other(format!("{}: {e}", tmp.display())))?;
        let mut h = Sha256::new();
        let mut total = 0u64;
        for i in 0..chunks {
            let sealed = client.get_chunk(blob_id, i as usize).await?;
            let plain = open_chunk(key, user_id, blob_id, i, &sealed).map_err(HubError::Other)?;
            total += plain.len() as u64;
            if total > size {
                return Err(HubError::Other("Datei größer als angekündigt".into()));
            }
            h.update(&plain);
            file.write_all(&plain)
                .map_err(|e| HubError::Other(format!("{}: {e}", tmp.display())))?;
        }
        file.sync_all()
            .map_err(|e| HubError::Other(format!("{}: {e}", tmp.display())))?;
        let sha: [u8; 32] = h.finalize().into();
        if total != size || self::blob_id(key, &sha) != blob_id {
            return Err(HubError::Other(
                "Heruntergeladene Datei passt nicht zur Kennung".into(),
            ));
        }
        Ok(hex(&sha))
    }
    .await;
    match result {
        Ok(sha) => std::fs::rename(&tmp, target).map(|_| sha).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            HubError::Other(format!("{}: {e}", target.display()))
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> [u8; KEY_LEN] {
        let mut k = [0u8; KEY_LEN];
        for (i, b) in k.iter_mut().enumerate() {
            *b = i as u8;
        }
        k
    }

    #[test]
    fn stueck_roundtrip_und_aad_bindung() {
        let k = key();
        let id = "a".repeat(64);
        let sealed = seal_chunk(&k, 7, &id, 3, b"Kapitel eins").unwrap();
        assert_eq!(sealed.len(), 24 + 12 + 16, "nonce ‖ chiffrat ‖ tag");
        assert_eq!(open_chunk(&k, 7, &id, 3, &sealed).unwrap(), b"Kapitel eins");
        assert!(open_chunk(&k, 7, &id, 4, &sealed).is_err(), "anderes Stück");
        assert!(
            open_chunk(&k, 8, &id, 3, &sealed).is_err(),
            "anderer Benutzer"
        );
        assert!(
            open_chunk(&k, 7, &"b".repeat(64), 3, &sealed).is_err(),
            "anderer Blob"
        );
        assert!(open_chunk(&k, 7, &id, 3, &sealed[..30]).is_err(), "zu kurz");
    }

    #[test]
    fn aad_wie_swift() {
        assert_eq!(chunk_aad(42, "ab", 0), b"42\x1fblob\x1fab\x1f0".to_vec());
    }

    #[test]
    fn blob_kennung_haengt_von_schluessel_und_inhalt_ab() {
        let k = key();
        let sha: [u8; 32] = Sha256::digest(b"Hallo").into();
        let a = blob_id(&k, &sha);
        assert!(valid_blob_id(&a));
        assert_eq!(a, blob_id(&k, &sha));
        let mut other = k;
        other[0] ^= 1;
        assert_ne!(a, blob_id(&other, &sha));
        // Bitgleich zur Definition in der Spec.
        let mut buf = k.to_vec();
        buf.extend_from_slice(b"lv-blob-v1");
        buf.extend_from_slice(&sha);
        assert_eq!(a, sha256_hex(&buf));
    }

    /// Bekannte Antwort für die Swift-Gegenprobe (`LibrarySyncFormat.sealChunk`
    /// mit fester Nonce): Schlüssel 00..1f, Benutzer 7, Nonce 40..57,
    /// Klartext „Hallo Buch“, Stück 0 des Blobs zu SHA-256(„Hallo Buch“).
    #[test]
    fn testvektor_fuer_swift() {
        let k = key();
        let sha: [u8; 32] = Sha256::digest("Hallo Buch".as_bytes()).into();
        let id = blob_id(&k, &sha);
        let mut nonce = [0u8; NONCE_LEN];
        for (i, b) in nonce.iter_mut().enumerate() {
            *b = 0x40 + i as u8;
        }
        let sealed = seal_chunk_with_nonce(&k, 7, &id, 0, "Hallo Buch".as_bytes(), &nonce).unwrap();
        println!("blob_id={id}\nsealed={}", hex(&sealed));
        assert_eq!(id, BLOB_ID_KAT);
        assert_eq!(hex(&sealed), SEALED_KAT);
    }

    const BLOB_ID_KAT: &str = "08d8fe6edc4415c512af3a369d59e5473fefa144043c3ca8d6ea8653210a7322";
    const SEALED_KAT: &str = "404142434445464748494a4b4c4d4e4f50515253545556579c58691cbfc03b63ec9ccb2edc16b2b3f35766f059626d3c4bbf";

    /// Gegen den Hub-Nachbau (`apps/apple/scripts/mock_voice_hub.py`):
    /// `LV_MOCK_HUB=http://127.0.0.1:8765 cargo test --lib -- --ignored blobs::tests::hub_nachbau`
    #[tokio::test]
    #[ignore]
    async fn hub_nachbau_hoch_und_runter() {
        let Ok(url) = std::env::var("LV_MOCK_HUB") else {
            return;
        };
        let client = HubClient::new(&url, "wai_pat_test");
        let k = key();
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("teil_001.mp3");
        let data: Vec<u8> = (0..(2 * CHUNK_PLAIN + 123))
            .map(|i| (i * 7 % 251) as u8)
            .collect();
        std::fs::write(&src, &data).unwrap();
        let sha: [u8; 32] = Sha256::digest(&data).into();
        let id = blob_id(&k, &sha);
        let size = data.len() as u64;
        // Budget für ein Stück: der Upload setzt im zweiten Anlauf fort.
        let mut budget = 1;
        let first = upload(
            &client,
            &k,
            7,
            &id,
            &Source::File(src.clone()),
            size,
            None,
            &mut budget,
        )
        .await
        .unwrap();
        assert_eq!(first, Upload::Later);
        let mut budget = u64::MAX;
        let second = upload(
            &client,
            &k,
            7,
            &id,
            &Source::File(src.clone()),
            size,
            None,
            &mut budget,
        )
        .await
        .unwrap();
        assert_eq!(second, Upload::Done);
        let target = dir.path().join("zurueck.mp3");
        download(&client, &k, 7, &id, size, &target).await.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), data);
        // Datei unterwegs geändert: kein Abschluss, der Blob bleibt offen.
        let neu = dir.path().join("teil_002.mp3");
        std::fs::write(&neu, b"Kapitel zwei").unwrap();
        let mut cache = BTreeMap::new();
        let mut b = u64::MAX;
        let mut st = stamp(&mut cache, &neu, &mut b).unwrap().unwrap();
        let id2 = blob_id_of(&k, &st).unwrap();
        st.mtime_ms -= 1;
        let mut budget = u64::MAX;
        let r = upload(
            &client,
            &k,
            7,
            &id2,
            &Source::File(neu.clone()),
            12,
            Some(&st),
            &mut budget,
        )
        .await;
        assert!(r.is_err(), "Stempel passt nicht: abbrechen");
        assert!(client
            .blob_info(&id2)
            .await
            .unwrap()
            .is_none_or(|i| !i.complete));
        // Gleiche Größe, anderer Inhalt: die Prüfsumme fällt auf.
        let st2 = stamp(&mut BTreeMap::new(), &neu, &mut u64::MAX)
            .unwrap()
            .unwrap();
        let wrong = blob_id(&k, &Sha256::digest(b"Kapitel drei").into());
        let r = upload(
            &client,
            &k,
            7,
            &wrong,
            &Source::File(neu),
            12,
            Some(&st2),
            &mut u64::MAX,
        )
        .await;
        assert!(r.is_err());
        assert!(client
            .blob_info(&wrong)
            .await
            .unwrap()
            .is_none_or(|i| !i.complete));

        // Falscher Benutzer: AAD passt nicht, nichts landet am Ziel.
        let other = dir.path().join("falsch.mp3");
        assert!(download(&client, &k, 8, &id, size, &other).await.is_err());
        assert!(!other.exists());
    }

    #[test]
    fn stueckzahl() {
        assert_eq!(chunk_count(0), 1);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_PLAIN as u64), 1);
        assert_eq!(chunk_count(CHUNK_PLAIN as u64 + 1), 2);
    }

    #[test]
    fn stuecke_einer_datei() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("teil.mp3");
        let data: Vec<u8> = (0..(CHUNK_PLAIN + 10)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&p, &data).unwrap();
        let src = Source::File(p.clone());
        let size = data.len() as u64;
        assert_eq!(src.read_chunk(0, size).unwrap(), data[..CHUNK_PLAIN]);
        assert_eq!(src.read_chunk(1, size).unwrap(), data[CHUNK_PLAIN..]);
        let bytes = Source::Bytes(data.clone());
        assert_eq!(bytes.read_chunk(1, size).unwrap().len(), 10);

        let mut cache = BTreeMap::new();
        let mut budget = 1;
        assert_eq!(
            stamp(&mut cache, &p, &mut budget).unwrap(),
            None,
            "Budget zu klein"
        );
        budget = u64::MAX;
        let s = stamp(&mut cache, &p, &mut budget).unwrap().unwrap();
        assert_eq!(s.sha256, sha256_hex(&data));
        let mut zero = 0;
        assert_eq!(
            stamp(&mut cache, &p, &mut zero).unwrap(),
            Some(s),
            "aus dem Zwischenspeicher, ohne Budget"
        );
    }
}
