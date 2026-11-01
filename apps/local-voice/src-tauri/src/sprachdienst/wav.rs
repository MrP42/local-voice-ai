//! WAV-Häppchen für Audio heraus (Rahmenart 3, höchstens `AUDIO_MAX` Bytes je Rahmen).
//!
//! Der Vertrag nennt Audio heraus "WAV-Häppchen". Dieses Modul legt fest, was das hier heißt:
//! Jedes Häppchen ist eine **in sich vollständige WAV-Datei** (44-Byte-Kopf, PCM16) mit
//! derselben Abtastrate und Kanalzahl wie die Quelle, höchstens `AUDIO_MAX` Bytes groß und an
//! Blockgrenzen geschnitten. Der Empfänger kann jedes Häppchen für sich abspielen oder die
//! Nutzdaten hintereinanderlegen.

use wai_voice_dienst::grenzen;

const KOPF: usize = 44;

#[derive(Debug, Clone, Copy)]
struct Format {
    kanaele: u16,
    rate: u32,
}

/// Findet `fmt ` und `data` in einer WAV-Datei (PCM16, auch `WAVE_FORMAT_EXTENSIBLE`).
fn lesen(wav: &[u8]) -> Result<(Format, &[u8]), String> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Err("kein WAV".into());
    }
    let mut pos = 12;
    let mut format: Option<Format> = None;
    while pos + 8 <= wav.len() {
        let id = &wav[pos..pos + 4];
        let laenge =
            u32::from_le_bytes([wav[pos + 4], wav[pos + 5], wav[pos + 6], wav[pos + 7]]) as usize;
        let start = pos + 8;
        let ende = start.saturating_add(laenge).min(wav.len());
        if id == b"fmt " {
            let f = &wav[start..ende];
            if f.len() < 16 {
                return Err("fmt zu kurz".into());
            }
            let tag = u16::from_le_bytes([f[0], f[1]]);
            let kanaele = u16::from_le_bytes([f[2], f[3]]);
            let rate = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
            let bits = u16::from_le_bytes([f[14], f[15]]);
            if !(tag == 1 || tag == 0xFFFE) || bits != 16 || kanaele == 0 || rate == 0 {
                return Err("nur PCM16".into());
            }
            format = Some(Format { kanaele, rate });
        } else if id == b"data" {
            let Some(format) = format else {
                return Err("data vor fmt".into());
            };
            return Ok((format, &wav[start..ende]));
        }
        // Chunks sind auf gerade Längen aufgefüllt.
        pos = start.saturating_add(laenge).saturating_add(laenge & 1);
    }
    Err("keine Nutzdaten".into())
}

fn kopf(format: Format, daten: usize) -> [u8; KOPF] {
    let block = u32::from(format.kanaele) * 2;
    let mut k = [0u8; KOPF];
    k[0..4].copy_from_slice(b"RIFF");
    k[4..8].copy_from_slice(&((36 + daten) as u32).to_le_bytes());
    k[8..12].copy_from_slice(b"WAVE");
    k[12..16].copy_from_slice(b"fmt ");
    k[16..20].copy_from_slice(&16u32.to_le_bytes());
    k[20..22].copy_from_slice(&1u16.to_le_bytes());
    k[22..24].copy_from_slice(&format.kanaele.to_le_bytes());
    k[24..28].copy_from_slice(&format.rate.to_le_bytes());
    k[28..32].copy_from_slice(&(format.rate * block).to_le_bytes());
    k[32..34].copy_from_slice(&(block as u16).to_le_bytes());
    k[34..36].copy_from_slice(&16u16.to_le_bytes());
    k[36..40].copy_from_slice(b"data");
    k[40..44].copy_from_slice(&(daten as u32).to_le_bytes());
    k
}

/// Zerlegt eine WAV-Datei in vollständige WAV-Häppchen von höchstens `AUDIO_MAX` Bytes.
pub fn haeppchen(wav: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let (format, daten) = lesen(wav)?;
    let block = usize::from(format.kanaele) * 2;
    let je = (grenzen::AUDIO_MAX - KOPF) / block * block;
    if je == 0 {
        return Err("zu viele Kanäle".into());
    }
    let nutz = daten.len() / block * block;
    Ok(daten[..nutz]
        .chunks(je)
        .map(|stueck| {
            let mut h = Vec::with_capacity(KOPF + stueck.len());
            h.extend_from_slice(&kopf(format, stueck.len()));
            h.extend_from_slice(stueck);
            h
        })
        .collect())
}

/// Eine WAV-Datei aus PCM16-Daten (für Tests und Attrappen).
pub fn wav_aus_pcm(pcm: &[u8], kanaele: u16, rate: u32) -> Vec<u8> {
    let mut w = kopf(Format { kanaele, rate }, pcm.len()).to_vec();
    w.extend_from_slice(pcm);
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schneidet_an_blockgrenzen_und_bleibt_gueltig() {
        let pcm: Vec<u8> = (0..70_000u32).map(|i| (i % 251) as u8).collect();
        let wav = wav_aus_pcm(&pcm, 1, 22_050);
        let teile = haeppchen(&wav).unwrap();
        assert!(teile.len() >= 3);
        let mut zusammen = Vec::new();
        for t in &teile {
            assert!(t.len() <= grenzen::AUDIO_MAX);
            let (f, d) = lesen(t).unwrap();
            assert_eq!((f.kanaele, f.rate), (1, 22_050));
            assert_eq!(d.len() % 2, 0);
            zusammen.extend_from_slice(d);
        }
        assert_eq!(zusammen, pcm, "nichts geht verloren, nichts kommt dazu");
    }

    #[test]
    fn stereo_bleibt_auf_dem_block() {
        let pcm = vec![7u8; 100_000];
        let teile = haeppchen(&wav_aus_pcm(&pcm, 2, 44_100)).unwrap();
        for t in &teile {
            assert_eq!((t.len() - KOPF) % 4, 0);
        }
    }

    #[test]
    fn unbrauchbares_wird_abgewiesen() {
        assert!(haeppchen(b"nichts").is_err());
        let mut f32_wav = wav_aus_pcm(&[0; 8], 1, 16_000);
        f32_wav[34] = 32;
        assert!(haeppchen(&f32_wav).is_err());
        assert_eq!(haeppchen(&wav_aus_pcm(&[], 1, 16_000)).unwrap().len(), 0);
    }

    #[test]
    fn fremde_chunks_vor_data_werden_uebersprungen() {
        let mut w = wav_aus_pcm(&[1, 0, 2, 0], 1, 16_000);
        let liste = b"LIST\x03\x00\x00\x00abc\x00";
        w.splice(36..36, liste.iter().copied());
        let t = haeppchen(&w).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(&t[0][KOPF..], &[1, 0, 2, 0]);
    }
}
