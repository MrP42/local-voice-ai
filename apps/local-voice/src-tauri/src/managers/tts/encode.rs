//! MP3-Kodierung fertiger WAV-Daten.
//!
//! Der Vorlese-Export baut seine Datei satzweise als WAV im Speicher
//! zusammen (siehe `TtsManager::speak_to_file`). Erst am Ende, wenn EIN durchgehendes
//! Tonstueck vorliegt, wird hier daraus MP3 — und nur so kann die Bitrate
//! ueberhaupt greifen: der fish-speech-Server kennt keinen Bitraten-Parameter
//! (`ServeTTSRequest` hat nur `format`), also muss die Kodierung bei uns
//! stattfinden.
//!
//! LAME kommt als vendored C-Quelle mit `mp3lame-encoder` mit; auf Windows
//! baut das ueber `cc`, ohne vorinstallierte Bibliothek.

use mp3lame_encoder::{Bitrate, Builder, FlushNoGap, InterleavedPcm, MonoPcm};

use crate::settings::clamp_tts_export_bitrate;

/// Die geklammerte Bitrate auf die LAME-Stufe abbilden.
///
/// `clamp_tts_export_bitrate` liefert garantiert einen der vier erlaubten
/// Werte; der Zweig darunter ist reine Absicherung gegen spaetere Erweiterung
/// der Liste.
fn lame_bitrate(kbps: u32) -> Bitrate {
    match clamp_tts_export_bitrate(kbps) {
        128 => Bitrate::Kbps128,
        256 => Bitrate::Kbps256,
        320 => Bitrate::Kbps320,
        _ => Bitrate::Kbps192,
    }
}

/// WAV-Bytes nach MP3 wandeln.
///
/// Abtastrate und Kanalzahl kommen aus dem WAV selbst — umgerechnet wird
/// nichts, die Datei soll klingen wie das, was vorher zusammengesetzt wurde.
/// Mehr als zwei Kanaele mischt LAME nicht; die werden vorher auf Mono
/// heruntergemischt, statt den Export scheitern zu lassen.
///
/// Fehler statt Panik: ein unlesbarer Eingang ist ein `Err`, denn diese
/// Funktion steht am Ende eines Exports, der dem Benutzer eine Meldung
/// schuldet.
pub fn wav_to_mp3(wav: &[u8], bitrate_kbps: u32) -> Result<Vec<u8>, String> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(wav))
        .map_err(|e| format!("WAV nicht lesbar: {e}"))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    // Wie in `decode_wav`: alles wird erst zu i16 vereinheitlicht, damit die
    // Kodierung nur einen Eingangstyp kennen muss.
    let samples: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .map(|s| {
                s.map(|v| (v * i16::MAX as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16)
            })
            .collect::<Result<_, _>>()
            .map_err(|e| format!("WAV beschaedigt: {e}"))?,
        hound::SampleFormat::Int => {
            let shift = (spec.bits_per_sample as i32 - 16).max(0);
            let scale = (1i64 << (16i32 - spec.bits_per_sample as i32).max(0)) as i32;
            reader
                .samples::<i32>()
                .map(|s| {
                    s.map(|v| ((v >> shift) * scale).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
                })
                .collect::<Result<_, _>>()
                .map_err(|e| format!("WAV beschaedigt: {e}"))?
        }
    };
    if samples.is_empty() {
        return Err("WAV ohne Ton".to_string());
    }

    // Mehrkanaliges auf Mono, Mono und Stereo bleiben, wie sie sind.
    let (samples, channels) = if channels > 2 {
        let mono: Vec<i16> = samples
            .chunks(channels)
            .map(|frame| (frame.iter().map(|s| *s as i32).sum::<i32>() / frame.len() as i32) as i16)
            .collect();
        (mono, 1usize)
    } else {
        (samples, channels)
    };

    let frames = samples.len() / channels;
    let mut encoder = Builder::new().ok_or_else(|| "LAME nicht initialisierbar".to_string())?;
    encoder
        .set_sample_rate(spec.sample_rate)
        .map_err(|e| format!("Abtastrate {} nicht kodierbar: {e}", spec.sample_rate))?;
    encoder
        .set_num_channels(channels as u8)
        .map_err(|e| format!("Kanalzahl {channels} nicht kodierbar: {e}"))?;
    encoder
        .set_brate(lame_bitrate(bitrate_kbps))
        .map_err(|e| format!("Bitrate nicht setzbar: {e}"))?;
    let mut encoder = encoder
        .build()
        .map_err(|e| format!("MP3-Kodierer nicht startbar: {e}"))?;

    let mut out: Vec<u8> = Vec::new();
    out.reserve(mp3lame_encoder::max_required_buffer_size(frames));
    if channels == 1 {
        encoder
            .encode_to_vec(MonoPcm(samples.as_slice()), &mut out)
            .map_err(|e| format!("MP3-Kodierung fehlgeschlagen: {e}"))?;
    } else {
        encoder
            .encode_to_vec(InterleavedPcm(samples.as_slice()), &mut out)
            .map_err(|e| format!("MP3-Kodierung fehlgeschlagen: {e}"))?;
    }
    out.reserve(7200);
    encoder
        .flush_to_vec::<FlushNoGap>(&mut out)
        .map_err(|e| format!("MP3-Abschluss fehlgeschlagen: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mono, 16 Bit, `rate` Hz — das Testmuster des Elternmoduls.
    fn sine_wav(rate: u32, samples: usize) -> Vec<u8> {
        super::super::test_support::sine_wav(rate, samples)
    }

    fn stereo_wav(rate: u32, frames: usize) -> Vec<u8> {
        let data_len = frames * 4;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 / 8.0).sin() * 12_000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
            out.extend_from_slice(&(-v).to_le_bytes());
        }
        out
    }

    fn looks_like_mp3(bytes: &[u8]) -> bool {
        bytes.starts_with(b"ID3") || bytes.first() == Some(&0xFF)
    }

    #[test]
    fn mono_wav_wird_zu_mp3() {
        let wav = sine_wav(44_100, 44_100);
        let mp3 = wav_to_mp3(&wav, 192).expect("Kodierung");
        assert!(looks_like_mp3(&mp3), "kein MP3-Kopf: {:?}", &mp3[..4]);
        assert!(mp3.len() > 1024, "zu kurz: {}", mp3.len());
    }

    #[test]
    fn stereo_wav_wird_zu_mp3() {
        let wav = stereo_wav(44_100, 44_100);
        let mp3 = wav_to_mp3(&wav, 192).expect("Kodierung");
        assert!(looks_like_mp3(&mp3));
        assert!(mp3.len() > 1024, "zu kurz: {}", mp3.len());
    }

    /// Der eigentliche Sinn der Einstellung: mehr Bitrate, mehr Bytes.
    #[test]
    fn hoehere_bitrate_gibt_groessere_datei() {
        let wav = sine_wav(44_100, 44_100);
        let klein = wav_to_mp3(&wav, 128).expect("128");
        let gross = wav_to_mp3(&wav, 320).expect("320");
        assert!(
            gross.len() > klein.len(),
            "320 kbit/s ({}) nicht groesser als 128 kbit/s ({})",
            gross.len(),
            klein.len()
        );
    }

    #[test]
    fn kaputter_eingang_gibt_fehler() {
        assert!(wav_to_mp3(b"nicht wirklich eine WAV-Datei", 192).is_err());
        assert!(wav_to_mp3(&[], 192).is_err());
    }

    #[test]
    fn bitrate_wird_auf_erlaubte_stufe_gezogen() {
        assert_eq!(clamp_tts_export_bitrate(100), 128);
        assert_eq!(clamp_tts_export_bitrate(999), 320);
        assert_eq!(clamp_tts_export_bitrate(0), 128);
        assert_eq!(clamp_tts_export_bitrate(192), 192);
        assert_eq!(clamp_tts_export_bitrate(256), 256);
    }
}
