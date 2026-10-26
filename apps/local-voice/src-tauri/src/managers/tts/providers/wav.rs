//! WAV-Helfer der Anbieterschicht: PCM in einen Container packen, einen
//! Header lesen (Rate, Kanäle, Dauer).
//!
//! Warum ein eigener Parser und nicht `hound` bzw. `wav_sample_rate` aus
//! `tts/mod.rs` (geprüft): `wav_sample_rate` liefert nur die Rate (und ist
//! privat). Wir brauchen zusätzlich Kanäle und Dauer, und wir müssen
//! Streaming-Header vertragen — Ollama und Phonon schreiben als Längenfeld
//! den Maximalwert `0xFFFFFFFF` (Quelle: provider-research.md, B). Dort ist
//! die echte Länge die Zahl der tatsächlich vorhandenen Bytes.

use super::contract::{AudioClip, AudioEncoding, AudioFormat, TtsError};

/// Was ein WAV-Header über seine Nutzdaten verrät.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavInfo {
    /// 1 = PCM, 3 = IEEE-Float (bei `WAVE_FORMAT_EXTENSIBLE` der Unterformat-Code).
    pub format_tag: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    /// Beginn der Nutzdaten im Blob.
    pub data_offset: usize,
    /// Tatsächlich vorhandene Nutzdaten (auf einen Blob-Rest und auf ganze
    /// Frames gekürzt).
    pub data_len: usize,
    pub frames: u64,
    /// Abgerundet.
    pub duration_ms: u64,
}

impl WavInfo {
    /// Die `AudioEncoding`, die zu den Nutzdaten passt (`None` bei 8/24 Bit usw.).
    pub fn encoding(&self) -> Option<AudioEncoding> {
        match (self.format_tag, self.bits_per_sample) {
            (1, 16) => Some(AudioEncoding::Pcm16Le),
            (3, 32) => Some(AudioEncoding::F32Le),
            _ => None,
        }
    }
}

/// Dauer in Millisekunden (abgerundet) aus Frames und Abtastrate.
pub fn duration_ms(frames: u64, sample_rate: u32) -> u64 {
    if sample_rate == 0 {
        return 0;
    }
    frames.saturating_mul(1000) / u64::from(sample_rate)
}

/// RIFF-Chunk-Größe (`36 + data_len`), falls sie in 32 Bit passt. Ein WAV
/// kann höchstens knapp 4 GiB Nutzdaten tragen; mehr ist ein Fehler, kein
/// stilles Überlaufen des Längenfelds.
pub fn checked_riff_len(data_len: usize) -> Result<u32, TtsError> {
    u64::try_from(data_len)
        .ok()
        .and_then(|n| n.checked_add(36))
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| TtsError::bad_request("Audio ist zu groß für eine WAV-Datei (über 4 GiB)"))
}

/// PCM-Rohdaten (`Pcm16Le` oder `F32Le`) in einen WAV-Container packen.
pub fn wav_from_pcm(
    encoding: AudioEncoding,
    sample_rate: u32,
    channels: u16,
    pcm: &[u8],
) -> Result<Vec<u8>, TtsError> {
    let (tag, bits): (u16, u16) = match encoding {
        AudioEncoding::Pcm16Le => (1, 16),
        AudioEncoding::F32Le => (3, 32),
        AudioEncoding::Wav => {
            return Err(TtsError::bad_request(
                "Wav ist kein Rohformat und lässt sich nicht erneut verpacken",
            ))
        }
    };
    if sample_rate == 0 || channels == 0 {
        return Err(TtsError::bad_request(
            "Abtastrate und Kanalzahl müssen größer als 0 sein",
        ));
    }
    let frame = usize::from(bits / 8) * usize::from(channels);
    let rest = pcm.len() % frame;
    if rest != 0 {
        return Err(TtsError::backend(format!(
            "PCM-Länge {} ist kein Vielfaches von {frame} Bytes, {rest} Byte(s) zu viel (abgeschnittener Strom?)",
            pcm.len()
        )));
    }
    let riff_len = checked_riff_len(pcm.len())?;
    let block_align =
        u16::try_from(frame).map_err(|_| TtsError::bad_request("Zu viele Kanäle für ein WAV"))?;
    let byte_rate = u32::try_from(u64::from(sample_rate) * frame as u64)
        .map_err(|_| TtsError::bad_request("Datenrate zu hoch für ein WAV"))?;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(pcm);
    Ok(out)
}

/// Header eines WAV-Blobs lesen. Verträgt Zusatz-Chunks vor `data`,
/// Streaming-Header (Längenfeld `0` oder `0xFFFFFFFF`) und abgeschnittene
/// Daten; panikt nie, auch nicht bei Müll.
pub fn parse_wav(bytes: &[u8]) -> Result<WavInfo, TtsError> {
    let bad = |what: &str| TtsError::backend(format!("Ungültiges WAV: {what}"));
    if bytes.len() < 12 {
        return Err(bad("zu kurz"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("kein RIFF/WAVE"));
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);

    // (Format-Code, Kanäle, Rate, Bits)
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut pos = 12usize;
    while pos.saturating_add(8) <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let declared = u32_at(bytes, pos + 4) as usize;
        let body = pos + 8;

        if id == b"fmt " {
            let end = body
                .checked_add(declared)
                .filter(|e| *e <= bytes.len())
                .ok_or_else(|| bad("fmt-Chunk abgeschnitten"))?;
            if declared < 16 {
                return Err(bad("fmt-Chunk zu kurz"));
            }
            let b = &bytes[body..end];
            let mut tag = u16_at(b, 0);
            if tag == 0xFFFE {
                // WAVE_FORMAT_EXTENSIBLE: der echte Code steht am Anfang der GUID.
                if declared < 40 {
                    return Err(bad("extensible fmt-Chunk zu kurz"));
                }
                tag = u16_at(b, 24);
            }
            fmt = Some((tag, u16_at(b, 2), u32_at(b, 4), u16_at(b, 14)));
        } else if id == b"data" {
            let (format_tag, channels, sample_rate, bits) =
                fmt.ok_or_else(|| bad("data-Chunk steht vor fmt"))?;
            if channels == 0 || sample_rate == 0 || bits == 0 {
                return Err(bad("Kanäle, Rate und Bits müssen größer als 0 sein"));
            }
            let frame = usize::from(channels) * usize::from(bits).div_ceil(8);
            let available = bytes.len() - body;
            // 0 und 0xFFFFFFFF schreiben Streaming-Quellen, wenn die Länge noch
            // unbekannt ist; zu große Längen stammen von abgeschnittenen Dateien.
            let raw = if declared == 0 || declared == u32::MAX as usize || declared > available {
                available
            } else {
                declared
            };
            let frames = raw / frame;
            return Ok(WavInfo {
                format_tag,
                channels,
                sample_rate,
                bits_per_sample: bits,
                data_offset: body,
                data_len: frames * frame,
                frames: frames as u64,
                duration_ms: duration_ms(frames as u64, sample_rate),
            });
        }

        // Nächster Chunk; Chunks sind auf gerade Längen aufgefüllt.
        let Some(next) = body
            .checked_add(declared)
            .and_then(|e| e.checked_add(declared & 1))
            .filter(|n| *n <= bytes.len())
        else {
            break;
        };
        pos = next;
    }
    Err(bad("kein data-Chunk"))
}

/// Nutzdaten eines Clips in der gewünschten Kodierung als Stream-Chunk:
/// `Wav` = der Container unverändert, ein Rohformat = nur die Nutzdaten,
/// wenn es dem Inhalt des Clips entspricht. Eine Umrechnung gibt es
/// bewusst nicht (BadRequest), damit nichts still verfälscht wird.
pub fn chunk_payload(
    clip: AudioClip,
    wanted: AudioEncoding,
) -> Result<(AudioFormat, Vec<u8>), TtsError> {
    if wanted == AudioEncoding::Wav {
        return Ok((clip.format, clip.data));
    }
    let info = parse_wav(&clip.data)?;
    if info.encoding() != Some(wanted) {
        return Err(TtsError::bad_request(format!(
            "Format {wanted:?} ist für diesen Clip nicht lieferbar (es wird nicht umgerechnet)"
        )));
    }
    let raw = clip.data[info.data_offset..info.data_offset + info.data_len].to_vec();
    Ok((
        AudioFormat {
            sample_rate: info.sample_rate,
            channels: info.channels,
            encoding: wanted,
        },
        raw,
    ))
}

impl AudioClip {
    /// Aus einem WAV-Blob; Format und Dauer kommen aus dem Header.
    pub fn from_wav(bytes: Vec<u8>) -> Result<Self, TtsError> {
        let info = parse_wav(&bytes)?;
        Ok(Self {
            format: AudioFormat {
                sample_rate: info.sample_rate,
                channels: info.channels,
                encoding: AudioEncoding::Wav,
            },
            duration_ms: info.duration_ms,
            data: bytes,
        })
    }

    /// Aus PCM-Rohdaten: packt sie in einen Container.
    pub fn from_pcm(
        encoding: AudioEncoding,
        sample_rate: u32,
        channels: u16,
        pcm: &[u8],
    ) -> Result<Self, TtsError> {
        Self::from_wav(wav_from_pcm(encoding, sample_rate, channels, pcm)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::providers::contract::TtsErrorKind;

    fn pcm16(frames: usize, channels: usize) -> Vec<u8> {
        (0..frames * channels)
            .flat_map(|i| ((i % 100) as i16 * 100).to_le_bytes())
            .collect()
    }

    /// Von Hand gebauter Blob mit frei wählbaren Chunks (Test-Gegenstück zum Parser).
    fn blob(riff_size: u32, chunks: &[(&[u8; 4], u32, Vec<u8>)]) -> Vec<u8> {
        let mut v = b"RIFF".to_vec();
        v.extend(riff_size.to_le_bytes());
        v.extend(b"WAVE");
        for (id, declared, body) in chunks {
            v.extend(*id);
            v.extend(declared.to_le_bytes());
            v.extend(body);
            if body.len() % 2 == 1 {
                v.push(0);
            }
        }
        v
    }

    fn fmt_body(tag: u16, ch: u16, rate: u32, bits: u16) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(tag.to_le_bytes());
        b.extend(ch.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * ch as u32 * bits as u32 / 8).to_le_bytes());
        b.extend((ch * bits / 8).to_le_bytes());
        b.extend(bits.to_le_bytes());
        b
    }

    #[test]
    fn pcm16_mono_rundreise_ergibt_header_und_dauer() {
        let pcm = pcm16(2400, 1);
        let wav = wav_from_pcm(AudioEncoding::Pcm16Le, 24_000, 1, &pcm).unwrap();
        assert_eq!(wav.len(), 44 + 4800);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(wav[4..8].try_into().unwrap()), 36 + 4800);
        let info = parse_wav(&wav).unwrap();
        assert_eq!(info.sample_rate, 24_000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(info.format_tag, 1);
        assert_eq!(info.data_offset, 44);
        assert_eq!(info.data_len, 4800);
        assert_eq!(info.frames, 2400);
        assert_eq!(info.duration_ms, 100);
        assert_eq!(info.encoding(), Some(AudioEncoding::Pcm16Le));
        assert_eq!(&wav[44..], &pcm[..]);
    }

    #[test]
    fn float_stereo_wird_als_f32le_erkannt() {
        let pcm = vec![0u8; 4 * 2 * 480];
        let wav = wav_from_pcm(AudioEncoding::F32Le, 48_000, 2, &pcm).unwrap();
        let info = parse_wav(&wav).unwrap();
        assert_eq!(info.format_tag, 3);
        assert_eq!(info.channels, 2);
        assert_eq!(info.bits_per_sample, 32);
        assert_eq!(info.frames, 480);
        assert_eq!(info.duration_ms, 10);
        assert_eq!(info.encoding(), Some(AudioEncoding::F32Le));
    }

    #[test]
    fn dauer_wird_abgerundet_und_ueberlauft_nicht() {
        assert_eq!(duration_ms(1, 24_000), 0);
        assert_eq!(duration_ms(24_000, 24_000), 1000);
        assert_eq!(duration_ms(24_001, 24_000), 1000);
        assert_eq!(duration_ms(0, 24_000), 0);
        assert_eq!(duration_ms(10, 0), 0, "Rate 0 darf nicht teilen");
        // Groesste Framezahl eines 4-GiB-WAV bei 1 Hz: kein Ueberlauf.
        assert_eq!(duration_ms(u32::MAX as u64, 1), u32::MAX as u64 * 1000);
    }

    #[test]
    fn ungueltige_eingaben_fuer_wav_from_pcm_sind_bad_request() {
        let e = wav_from_pcm(AudioEncoding::Wav, 24_000, 1, &[0, 0]).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        let e = wav_from_pcm(AudioEncoding::Pcm16Le, 0, 1, &[0, 0]).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        let e = wav_from_pcm(AudioEncoding::Pcm16Le, 24_000, 0, &[0, 0]).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
        // Halbes Sample (abgebrochener Strom) wird nicht still abgeschnitten.
        let e = wav_from_pcm(AudioEncoding::Pcm16Le, 24_000, 1, &[0, 0, 0]).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::Backend);
        // Stereo: ein Frame = 4 Bytes.
        assert!(wav_from_pcm(AudioEncoding::Pcm16Le, 24_000, 2, &[0; 6]).is_err());
    }

    #[test]
    fn leere_nutzdaten_ergeben_einen_gueltigen_leeren_clip() {
        let wav = wav_from_pcm(AudioEncoding::Pcm16Le, 24_000, 1, &[]).unwrap();
        let info = parse_wav(&wav).unwrap();
        assert_eq!((info.data_len, info.frames, info.duration_ms), (0, 0, 0));
    }

    #[test]
    fn die_4gib_grenze_wird_ohne_riesigen_puffer_geprueft() {
        assert_eq!(checked_riff_len(0).unwrap(), 36);
        assert_eq!(checked_riff_len(u32::MAX as usize - 36).unwrap(), u32::MAX);
        assert!(checked_riff_len(u32::MAX as usize - 35).is_err());
        assert!(checked_riff_len(usize::MAX).is_err());
    }

    #[test]
    fn zusatzchunks_vor_data_werden_uebersprungen_auch_mit_padbyte() {
        let data = pcm16(240, 1);
        let wav = blob(
            0,
            &[
                (b"LIST", 5, b"INFOx".to_vec()), // ungerade Länge -> Padbyte
                (b"fmt ", 16, fmt_body(1, 1, 24_000, 16)),
                (b"data", data.len() as u32, data.clone()),
            ],
        );
        let info = parse_wav(&wav).unwrap();
        assert_eq!(info.data_len, 480);
        assert_eq!(info.duration_ms, 10);
        assert_eq!(
            &wav[info.data_offset..info.data_offset + info.data_len],
            &data[..]
        );
    }

    #[test]
    fn streaming_header_mit_maximalen_laengen_zaehlt_die_vorhandenen_bytes() {
        // So schreiben Ollama/Phonon: riff = data = 0xFFFFFFFF.
        let data = pcm16(2400, 1);
        let wav = blob(
            u32::MAX,
            &[
                (b"fmt ", 16, fmt_body(1, 1, 24_000, 16)),
                (b"data", u32::MAX, data),
            ],
        );
        let info = parse_wav(&wav).unwrap();
        assert_eq!(info.data_len, 4800);
        assert_eq!(info.duration_ms, 100);
        // Auch Laenge 0 gilt als "unbekannt".
        let data = pcm16(2400, 1);
        let wav = blob(
            0,
            &[
                (b"fmt ", 16, fmt_body(1, 1, 24_000, 16)),
                (b"data", 0, data),
            ],
        );
        assert_eq!(parse_wav(&wav).unwrap().duration_ms, 100);
    }

    #[test]
    fn abgeschnittene_daten_und_halbe_frames_werden_gekuerzt() {
        // Stereo, ein Frame = 4 Bytes: 398 vorhandene Bytes = 99 ganze Frames + 2 Bytes.
        let data = pcm16(100, 2);
        let wav = blob(
            0,
            &[
                (b"fmt ", 16, fmt_body(1, 2, 24_000, 16)),
                (b"data", 10_000, data[..data.len() - 2].to_vec()),
            ],
        );
        let info = parse_wav(&wav).unwrap();
        assert_eq!(info.data_len, 396);
        assert_eq!(info.frames, 99);
    }

    #[test]
    fn extensible_header_liefert_den_unterformat_code() {
        for (sub, bits, want) in [
            (1u16, 16u16, AudioEncoding::Pcm16Le),
            (3u16, 32u16, AudioEncoding::F32Le),
        ] {
            let mut fmt = fmt_body(0xFFFE, 1, 24_000, bits);
            fmt.extend(22u16.to_le_bytes()); // cbSize
            fmt.extend(bits.to_le_bytes()); // valid bits
            fmt.extend(0u32.to_le_bytes()); // channel mask
            fmt.extend(sub.to_le_bytes()); // Unterformat (Anfang der GUID)
            fmt.extend([0u8; 14]); // Rest der GUID
            assert_eq!(fmt.len(), 40);
            let wav = blob(0, &[(b"fmt ", 40, fmt), (b"data", 4, vec![0, 0, 0, 0])]);
            let info = parse_wav(&wav).unwrap();
            assert_eq!(info.format_tag, sub);
            assert_eq!(info.encoding(), Some(want));
        }
    }

    #[test]
    fn kaputte_wavs_sind_fehler_und_keine_panik() {
        let good = wav_from_pcm(AudioEncoding::Pcm16Le, 24_000, 1, &pcm16(50, 1)).unwrap();
        // Jedes Präfix: Ok oder Err, nie Panik.
        for n in 0..good.len() {
            let _ = parse_wav(&good[..n]);
        }
        assert!(parse_wav(&good[..11]).is_err());
        assert!(parse_wav(&good[..40]).is_err(), "fmt da, aber kein data");
        assert!(parse_wav(b"").is_err());
        assert!(parse_wav(b"RIFX\0\0\0\0WAVE").is_err());
        assert!(parse_wav(b"not a wav at all, definitely not").is_err());
        // Rate 0 und Kanäle 0 sind keine gültigen Header.
        let bad_rate = blob(
            0,
            &[(b"fmt ", 16, fmt_body(1, 1, 0, 16)), (b"data", 0, vec![])],
        );
        assert!(parse_wav(&bad_rate).is_err());
        let bad_ch = blob(
            0,
            &[
                (b"fmt ", 16, fmt_body(1, 0, 24_000, 16)),
                (b"data", 0, vec![]),
            ],
        );
        assert!(parse_wav(&bad_ch).is_err());
        let short_fmt = blob(0, &[(b"fmt ", 8, vec![0; 8]), (b"data", 0, vec![])]);
        assert!(parse_wav(&short_fmt).is_err());
        // Ein Chunk, dessen Länge hinter das Blobende zeigt, endet in einem Fehler.
        let lying = blob(0, &[(b"junk", u32::MAX, vec![1, 2, 3, 4])]);
        assert!(parse_wav(&lying).is_err());
        // Pseudozufälliger Müll (deterministisch): nie Panik.
        let mut x: u32 = 0x1234_5678;
        for _ in 0..200 {
            let mut v = b"RIFF\0\0\0\0WAVE".to_vec();
            for _ in 0..64 {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                v.push((x >> 24) as u8);
            }
            let _ = parse_wav(&v);
        }
    }

    #[test]
    fn audio_clip_from_wav_und_from_pcm_fuellen_format_und_dauer() {
        let wav = wav_from_pcm(AudioEncoding::Pcm16Le, 22_050, 1, &pcm16(2205, 1)).unwrap();
        let clip = AudioClip::from_wav(wav.clone()).unwrap();
        assert_eq!(
            clip.format,
            AudioFormat {
                sample_rate: 22_050,
                channels: 1,
                encoding: AudioEncoding::Wav
            }
        );
        assert_eq!(clip.duration_ms, 100);
        assert_eq!(clip.data, wav);
        let clip2 =
            AudioClip::from_pcm(AudioEncoding::Pcm16Le, 22_050, 1, &pcm16(2205, 1)).unwrap();
        assert_eq!(clip2, clip);
        assert!(AudioClip::from_wav(vec![1, 2, 3]).is_err());
    }

    #[test]
    fn chunk_payload_liefert_container_oder_rohdaten_aber_rechnet_nie_um() {
        let pcm = pcm16(240, 1);
        let clip = AudioClip::from_pcm(AudioEncoding::Pcm16Le, 24_000, 1, &pcm).unwrap();
        let (f, d) = chunk_payload(clip.clone(), AudioEncoding::Wav).unwrap();
        assert_eq!(f.encoding, AudioEncoding::Wav);
        assert_eq!(d, clip.data);
        let (f, d) = chunk_payload(clip.clone(), AudioEncoding::Pcm16Le).unwrap();
        assert_eq!(f.encoding, AudioEncoding::Pcm16Le);
        assert_eq!((f.sample_rate, f.channels), (24_000, 1));
        assert_eq!(d, pcm);
        let e = chunk_payload(clip.clone(), AudioEncoding::F32Le).unwrap_err();
        assert_eq!(e.kind, TtsErrorKind::BadRequest);
    }
}
