//! Eingebettete Cover aus Hörbuch-Dateien lesen, ohne zusätzliche Bibliothek
//! und ohne Netz: ID3v2 (`APIC`/`PIC`, MP3), MP4-Atome (`covr`, M4B/M4A/AAC)
//! und FLAC (`PICTURE`). Gelesen wird nur der Kopf bzw. das `moov`-Atom, nie
//! die ganze Datei. Ergebnis sind die rohen Bildbytes (JPEG/PNG/WebP); was
//! kein Bild ist, fällt weg.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Obergrenze für einen gelesenen Kopf (ID3-Tag, `moov`-Atom).
const MAX_HEADER: u64 = 64 * 1024 * 1024;

/// Ist das ein Bild, das als Cover taugt?
fn is_image(b: &[u8]) -> bool {
    b.len() >= 512
        && (b.starts_with(&[0xFF, 0xD8, 0xFF])
            || b.starts_with(b"\x89PNG\r\n\x1a\n")
            || (b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP"))
}

/// Eingebettetes Cover einer Audiodatei, falls vorhanden.
pub fn embedded_cover(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 12];
    let n = file.read(&mut head).ok()?;
    let head = &head[..n];
    let found = if head.starts_with(b"ID3") {
        id3_cover(&mut file)
    } else if head.starts_with(b"fLaC") {
        flac_cover(&mut file)
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        mp4_cover(&mut file)
    } else {
        None
    };
    found.filter(|b| is_image(b))
}

// ---------------------------------------------------------------------------
// ID3v2
// ---------------------------------------------------------------------------

fn syncsafe(b: &[u8]) -> usize {
    b.iter()
        .fold(0usize, |acc, x| (acc << 7) | (*x as usize & 0x7F))
}

fn be(b: &[u8]) -> usize {
    b.iter().fold(0usize, |acc, x| (acc << 8) | *x as usize)
}

/// `FF 00` → `FF` (Unsynchronisation, ID3v2.3 über den ganzen Tag).
fn unsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        out.push(b[i]);
        if b[i] == 0xFF && b.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

fn id3_cover(file: &mut File) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0u8; 10];
    file.read_exact(&mut header).ok()?;
    let size = syncsafe(&header[6..10]) as u64;
    if size > MAX_HEADER {
        return None;
    }
    let mut tag = vec![0u8; size as usize];
    file.read_exact(&mut tag).ok()?;
    parse_id3(header[3], header[5], &tag)
}

/// Bild aus dem Tag-Inhalt (ohne die 10 Kopfbytes). Ein Vorderseiten-Bild
/// (Typ 3) gewinnt, sonst das erste.
pub fn parse_id3(version: u8, flags: u8, tag: &[u8]) -> Option<Vec<u8>> {
    let tag = if version <= 3 && flags & 0x80 != 0 {
        unsync(tag)
    } else {
        tag.to_vec()
    };
    let mut pos = 0usize;
    if flags & 0x40 != 0 && version >= 3 {
        // Erweiterter Kopf: v2.3 Größe ohne sich selbst, v2.4 syncsafe mit.
        let ext = tag.get(0..4)?;
        pos = if version == 3 {
            be(ext) + 4
        } else {
            syncsafe(ext)
        };
    }
    let (id_len, size_len, head_len) = if version == 2 { (3, 3, 6) } else { (4, 4, 10) };
    let mut first: Option<Vec<u8>> = None;
    while pos + head_len <= tag.len() {
        let id = &tag[pos..pos + id_len];
        if id.iter().all(|b| *b == 0) {
            break;
        }
        let raw = &tag[pos + id_len..pos + id_len + size_len];
        let size = if version == 4 { syncsafe(raw) } else { be(raw) };
        let start = pos + head_len;
        let end = start.checked_add(size)?;
        if end > tag.len() {
            break;
        }
        if id == b"APIC" || id == b"PIC" {
            if let Some((kind, image)) = picture_frame(version, &tag[start..end]) {
                if kind == 3 {
                    return Some(image);
                }
                first.get_or_insert(image);
            }
        }
        pos = end;
    }
    first
}

/// (Bildtyp, Bytes) aus einem `APIC`/`PIC`-Rahmen.
fn picture_frame(version: u8, p: &[u8]) -> Option<(u8, Vec<u8>)> {
    let enc = *p.first()?;
    let after_mime = if version == 2 {
        4 // drei Zeichen Format („JPG“)
    } else {
        1 + p.get(1..)?.iter().position(|b| *b == 0)? + 1
    };
    let kind = *p.get(after_mime)?;
    let desc = after_mime + 1;
    let rest = p.get(desc..)?;
    let data_at = if enc == 1 || enc == 2 {
        // UTF-16: zwei Nullbytes auf gerader Stelle beenden die Beschreibung.
        (0..rest.len().saturating_sub(1))
            .step_by(2)
            .find(|i| rest[*i] == 0 && rest[i + 1] == 0)
            .map(|i| i + 2)
    } else {
        rest.iter().position(|b| *b == 0).map(|i| i + 1)
    };
    let data = match data_at {
        Some(at) if is_image(&rest[at..]) => &rest[at..],
        // Kaputte Beschreibung: nach der Bildsignatur suchen.
        _ => {
            let at = find_image_start(rest)?;
            &rest[at..]
        }
    };
    Some((kind, data.to_vec()))
}

fn find_image_start(b: &[u8]) -> Option<usize> {
    (0..b.len()).find(|i| is_image(&b[*i..]))
}

// ---------------------------------------------------------------------------
// MP4 (M4B, M4A)
// ---------------------------------------------------------------------------

fn mp4_cover(file: &mut File) -> Option<Vec<u8>> {
    let len = file.metadata().ok()?.len();
    let mut pos = 0u64;
    // Nur oberste Ebene ablaufen; `moov` steht vorn oder hinten.
    while pos + 8 <= len {
        file.seek(SeekFrom::Start(pos)).ok()?;
        let mut h = [0u8; 8];
        file.read_exact(&mut h).ok()?;
        let mut size = be(&h[0..4]) as u64;
        let mut header = 8u64;
        if size == 1 {
            let mut ext = [0u8; 8];
            file.read_exact(&mut ext).ok()?;
            size = u64::from_be_bytes(ext);
            header = 16;
        } else if size == 0 {
            size = len - pos;
        }
        if size < header {
            return None;
        }
        if &h[4..8] == b"moov" {
            let body = size - header;
            if body > MAX_HEADER {
                return None;
            }
            let mut moov = vec![0u8; body as usize];
            file.read_exact(&mut moov).ok()?;
            return parse_moov(&moov);
        }
        pos = pos.checked_add(size)?;
    }
    None
}

/// Kinder eines Atoms: (Typ, Inhalt).
fn atoms(b: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 8 <= b.len() {
        let size = be(&b[pos..pos + 4]);
        let kind = &b[pos + 4..pos + 8];
        let (start, end) = match size {
            0 => (pos + 8, b.len()),
            1 => {
                let Some(ext) = b.get(pos + 8..pos + 16) else {
                    break;
                };
                let s = u64::from_be_bytes(ext.try_into().ok().unwrap_or_default()) as usize;
                (pos + 16, pos.saturating_add(s))
            }
            s => (pos + 8, pos.saturating_add(s)),
        };
        if end > b.len() || end < start {
            break;
        }
        out.push((kind, &b[start..end]));
        pos = end;
    }
    out
}

fn child<'a>(b: &'a [u8], kind: &[u8]) -> Option<&'a [u8]> {
    atoms(b)
        .into_iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, v)| v)
}

/// `moov/udta/meta/ilst/covr/data` (meta hat vier Bytes Version/Flags).
pub fn parse_moov(moov: &[u8]) -> Option<Vec<u8>> {
    let udta = child(moov, b"udta")?;
    let meta = child(udta, b"meta")?;
    // Manche Schreiber lassen Version/Flags weg (QuickTime-Stil).
    let meta_body = if meta.get(4..8) == Some(b"hdlr") {
        meta
    } else {
        meta.get(4..)?
    };
    let ilst = child(meta_body, b"ilst")?;
    let covr = child(ilst, b"covr")?;
    atoms(covr)
        .into_iter()
        .filter(|(k, _)| *k == b"data")
        // Acht Bytes Typ und Gebietsschema vor dem Bild.
        .filter_map(|(_, v)| v.get(8..))
        .find(|img| is_image(img))
        .map(<[u8]>::to_vec)
}

// ---------------------------------------------------------------------------
// FLAC
// ---------------------------------------------------------------------------

fn flac_cover(file: &mut File) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(4)).ok()?;
    let mut read = 0u64;
    let mut first: Option<Vec<u8>> = None;
    loop {
        let mut h = [0u8; 4];
        file.read_exact(&mut h).ok()?;
        let last = h[0] & 0x80 != 0;
        let kind = h[0] & 0x7F;
        let len = be(&h[1..4]) as u64;
        read += len + 4;
        if read > MAX_HEADER {
            return first;
        }
        if kind == 6 {
            let mut block = vec![0u8; len as usize];
            file.read_exact(&mut block).ok()?;
            if let Some((t, img)) = parse_flac_picture(&block) {
                if t == 3 {
                    return Some(img);
                }
                first.get_or_insert(img);
            }
        } else {
            file.seek(SeekFrom::Current(len as i64)).ok()?;
        }
        if last {
            return first;
        }
    }
}

/// (Bildtyp, Bytes) aus einem FLAC-`PICTURE`-Block.
pub fn parse_flac_picture(b: &[u8]) -> Option<(u8, Vec<u8>)> {
    let u32_at = |at: usize| b.get(at..at + 4).map(be);
    let kind = u32_at(0)? as u8;
    let mime_len = u32_at(4)?;
    let desc_at = 8 + mime_len;
    let desc_len = u32_at(desc_at)?;
    let data_len_at = desc_at + 4 + desc_len + 16;
    let data_len = u32_at(data_len_at)?;
    let data = b.get(data_len_at + 4..data_len_at + 4 + data_len)?;
    Some((kind, data.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg() -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.resize(600, 7);
        v
    }

    fn atom(kind: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn reads_id3v23_front_cover() {
        let mut frame = vec![0u8]; // Latin-1
        frame.extend_from_slice(b"image/jpeg\0");
        frame.push(3); // Vorderseite
        frame.extend_from_slice(b"Cover\0");
        frame.extend_from_slice(&jpeg());
        let mut tag = b"TIT2".to_vec();
        tag.extend_from_slice(&5u32.to_be_bytes());
        tag.extend_from_slice(&[0, 0]);
        tag.extend_from_slice(&[0, b'E', b'f', b'f', b'i']);
        tag.extend_from_slice(b"APIC");
        tag.extend_from_slice(&(frame.len() as u32).to_be_bytes());
        tag.extend_from_slice(&[0, 0]);
        tag.extend_from_slice(&frame);
        tag.extend_from_slice(&[0; 20]);
        assert_eq!(parse_id3(3, 0, &tag), Some(jpeg()));
    }

    #[test]
    fn reads_id3_utf16_description_and_prefers_front() {
        let other = {
            let mut f = vec![0u8];
            f.extend_from_slice(b"image/png\0");
            f.push(0); // sonstiges Bild
            f.push(0);
            let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
            png.resize(600, 1);
            f.extend_from_slice(&png);
            f
        };
        let front = {
            let mut f = vec![1u8]; // UTF-16
            f.extend_from_slice(b"image/jpeg\0");
            f.push(3);
            f.extend_from_slice(&[0xFF, 0xFE, b'C', 0, 0, 0]);
            f.extend_from_slice(&jpeg());
            f
        };
        let mut tag = Vec::new();
        for f in [&other, &front] {
            tag.extend_from_slice(b"APIC");
            tag.extend_from_slice(&(f.len() as u32).to_be_bytes());
            tag.extend_from_slice(&[0, 0]);
            tag.extend_from_slice(f);
        }
        assert_eq!(parse_id3(3, 0, &tag), Some(jpeg()));
    }

    #[test]
    fn reads_mp4_covr() {
        let mut data_body = vec![0, 0, 0, 13, 0, 0, 0, 0];
        data_body.extend_from_slice(&jpeg());
        let covr = atom(b"covr", &atom(b"data", &data_body));
        let ilst = atom(b"ilst", &covr);
        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend_from_slice(&atom(b"hdlr", &[0; 25]));
        meta_body.extend_from_slice(&ilst);
        let udta = atom(b"udta", &atom(b"meta", &meta_body));
        let mut moov = atom(b"mvhd", &[0; 100]);
        moov.extend_from_slice(&udta);
        assert_eq!(parse_moov(&moov), Some(jpeg()));
        assert_eq!(parse_moov(&atom(b"mvhd", &[0; 10])), None);
    }

    #[test]
    fn reads_mp4_file_with_moov_at_end() {
        let mut data_body = vec![0, 0, 0, 13, 0, 0, 0, 0];
        data_body.extend_from_slice(&jpeg());
        let ilst = atom(b"ilst", &atom(b"covr", &atom(b"data", &data_body)));
        let mut meta_body = vec![0, 0, 0, 0];
        meta_body.extend_from_slice(&ilst);
        let moov = atom(b"moov", &atom(b"udta", &atom(b"meta", &meta_body)));
        let mut file = atom(b"ftyp", b"M4B \0\0\0\0");
        file.extend_from_slice(&atom(b"mdat", &[9; 4096]));
        file.extend_from_slice(&moov);
        let dir = std::env::temp_dir().join(format!("lva-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("buch.m4b");
        std::fs::write(&p, &file).unwrap();
        assert_eq!(embedded_cover(&p), Some(jpeg()));
        std::fs::write(&p, b"ID3\x03\0\0\0\0\0\0").unwrap();
        assert_eq!(embedded_cover(&p), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_flac_picture() {
        let mut b = 3u32.to_be_bytes().to_vec();
        b.extend_from_slice(&10u32.to_be_bytes());
        b.extend_from_slice(b"image/jpeg");
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&[0; 16]);
        b.extend_from_slice(&(jpeg().len() as u32).to_be_bytes());
        b.extend_from_slice(&jpeg());
        assert_eq!(parse_flac_picture(&b), Some((3, jpeg())));
        assert_eq!(parse_flac_picture(&b[..30]), None);
    }

    #[test]
    fn garbage_never_panics() {
        for n in 0..64u8 {
            let junk: Vec<u8> = (0..n as usize * 7)
                .map(|i| (i as u8).wrapping_mul(n))
                .collect();
            let _ = parse_id3(2, 0, &junk);
            let _ = parse_id3(3, 0x40, &junk);
            let _ = parse_id3(4, 0x80, &junk);
            let _ = parse_moov(&junk);
            let _ = parse_flac_picture(&junk);
        }
    }
}
