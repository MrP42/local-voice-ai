//! Unigram-Zerleger (SentencePiece, XLM-R) für `multilingual-e5-small`, gelesen aus `tokenizer.json`.
//!
//! Nachgebaut ist, was die Hugging-Face-Pipeline dieses Modells tut: Normalisierung (`Precompiled`
//! nmt_nfkc, hier als NFKC mit Steuerzeichen raus und Leerraum zu einem Leerzeichen), Metaspace (`▁` vor
//! jedem Wort), Viterbi über die Stücke des Vokabulars, unbekannte Zeichen als ein zusammengefasstes
//! `<unk>`, Rahmen `<s> … </s>`. Ein eigener Zerleger statt der Bibliothek `tokenizers` vermeidet eine
//! große neue Abhängigkeit; der ignorierte Referenztest prüft die IDs gegen die Bibliothek.

use std::collections::HashMap;
use unicode_normalization::UnicodeNormalization;

/// Längstes Stück im Vokabular (in Zeichen), gekappt; XLM-R hat höchstens 16.
const MAX_STUECK: usize = 24;
/// Abschlag für unbekannte Zeichen (wie `tokenizers`: kleinster Wert − 10).
const UNK_ABSCHLAG: f64 = 10.0;

pub const BOS: i64 = 0;
pub const PAD: i64 = 1;
pub const EOS: i64 = 2;

#[derive(Debug)]
pub struct Zerleger {
    stuecke: HashMap<String, (i64, f64)>,
    unk: i64,
    unk_wert: f64,
    max_len: usize,
}

#[derive(serde::Deserialize)]
struct Datei {
    model: Modell,
}

#[derive(serde::Deserialize)]
struct Modell {
    #[serde(rename = "type")]
    art: String,
    unk_id: Option<i64>,
    vocab: Vec<(String, f64)>,
}

impl Zerleger {
    /// Aus dem Inhalt von `tokenizer.json` (nur `model.type = Unigram`).
    pub fn aus_json(bytes: &[u8]) -> Result<Self, String> {
        let d: Datei = serde_json::from_slice(bytes).map_err(|_| "zerleger.format".to_string())?;
        if d.model.art != "Unigram" || d.model.vocab.is_empty() {
            return Err("zerleger.format".into());
        }
        Ok(Self::neu(d.model.vocab, d.model.unk_id.unwrap_or(3)))
    }

    /// Aus einem Vokabular (Stück, Wert); Position = ID.
    pub fn neu(vocab: Vec<(String, f64)>, unk: i64) -> Self {
        let unk_wert = vocab.iter().map(|(_, w)| *w).fold(f64::INFINITY, f64::min) - UNK_ABSCHLAG;
        let max_len = vocab
            .iter()
            .map(|(s, _)| s.chars().count())
            .max()
            .unwrap_or(1)
            .min(MAX_STUECK);
        let stuecke = vocab
            .into_iter()
            .enumerate()
            // Sonderzeichen (`<s>`, `<pad>`, …) kommen im Text nie als Stück vor.
            .filter(|(i, (s, _))| !(s.starts_with('<') && s.ends_with('>') && *i < 4))
            .map(|(i, (s, w))| (s, (i as i64, w)))
            .collect();
        Self {
            stuecke,
            unk,
            unk_wert,
            max_len,
        }
    }

    /// Normalisierter Text, in Wörter mit `▁` zerlegt.
    fn woerter(text: &str) -> Vec<String> {
        let norm: String = text
            .nfkc()
            .filter_map(|c| {
                if c.is_whitespace() {
                    Some(' ')
                } else if c.is_control() || c == '\u{200b}' || c == '\u{feff}' {
                    None
                } else {
                    Some(c)
                }
            })
            .collect();
        norm.split(' ')
            .filter(|w| !w.is_empty())
            .map(|w| format!("\u{2581}{w}"))
            .collect()
    }

    /// Viterbi über ein Wort: beste Zerlegung nach Summe der Werte.
    fn wort(&self, wort: &str, aus: &mut Vec<i64>) {
        let grenzen: Vec<usize> = wort
            .char_indices()
            .map(|(b, _)| b)
            .chain(std::iter::once(wort.len()))
            .collect();
        let n = grenzen.len() - 1;
        // bester[i] = (Wert, Start, ID) des besten Wegs bis Zeichen i.
        let mut bester: Vec<Option<(f64, usize, i64)>> = vec![None; n + 1];
        bester[0] = Some((0.0, 0, -1));
        for start in 0..n {
            let Some((basis, _, _)) = bester[start] else {
                continue;
            };
            let mut einzel = false;
            for laenge in 1..=self.max_len.min(n - start) {
                let ende = start + laenge;
                if let Some(&(id, w)) = self.stuecke.get(&wort[grenzen[start]..grenzen[ende]]) {
                    einzel |= laenge == 1;
                    let neu = basis + w;
                    if bester[ende].is_none_or(|(alt, _, _)| neu > alt) {
                        bester[ende] = Some((neu, start, id));
                    }
                }
            }
            if !einzel {
                let neu = basis + self.unk_wert;
                if bester[start + 1].is_none_or(|(alt, _, _)| neu > alt) {
                    bester[start + 1] = Some((neu, start, self.unk));
                }
            }
        }
        let mut ids = Vec::new();
        let mut i = n;
        while i > 0 {
            let Some((_, start, id)) = bester[i] else {
                break;
            };
            ids.push(id);
            i = start;
        }
        ids.reverse();
        // Aufeinanderfolgende `<unk>` zu einem zusammenfassen.
        for id in ids {
            if id == self.unk && aus.last() == Some(&self.unk) {
                continue;
            }
            aus.push(id);
        }
    }

    /// IDs mit Rahmen `<s> … </s>`, höchstens `max` insgesamt.
    pub fn kodieren(&self, text: &str, max: usize) -> Vec<i64> {
        let mut ids = vec![BOS];
        for w in Self::woerter(text) {
            self.wort(&w, &mut ids);
            if ids.len() >= max.saturating_sub(1) {
                break;
            }
        }
        ids.truncate(max.saturating_sub(1).max(1));
        ids.push(EOS);
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mini() -> Zerleger {
        let v = [
            ("<s>", 0.0),
            ("<pad>", 0.0),
            ("</s>", 0.0),
            ("<unk>", 0.0),
            ("▁", -2.0),
            ("▁Ver", -4.0),
            ("trag", -4.0),
            ("▁Vertrag", -5.0),
            ("s", -3.0),
            ("▁lauf", -4.0),
            ("t", -3.0),
            ("V", -6.0),
            ("e", -6.0),
            ("r", -6.0),
        ];
        Zerleger::neu(v.iter().map(|(s, w)| (s.to_string(), *w)).collect(), 3)
    }

    #[test]
    fn viterbi_waehlt_das_wahrscheinlichste_stueck() {
        let z = mini();
        // „▁Vertrag“ (−5) schlägt „▁Ver“ + „trag“ (−8).
        assert_eq!(z.kodieren("Vertrag", 512), [BOS, 7, EOS]);
        assert_eq!(z.kodieren("Vertrags läuft", 512)[..3], [BOS, 7, 8]);
    }

    #[test]
    fn unbekannte_zeichen_werden_zu_einem_unk() {
        let z = mini();
        // „läuft“: „▁l“ unbekannt … → ▁ + unk (zusammengefasst) + t
        let ids = z.kodieren("Vertrag ßßß", 512);
        assert_eq!(ids, [BOS, 7, 4, 3, EOS]);
    }

    #[test]
    fn leerraum_und_nfkc_werden_normalisiert() {
        assert_eq!(
            Zerleger::woerter("  a\u{00a0}\tb\n\u{fb01} "),
            ["▁a", "▁b", "▁fi"]
        );
    }

    #[test]
    fn laenge_ist_begrenzt_mit_rahmen() {
        let z = mini();
        let ids = z.kodieren(&"Vertrag ".repeat(100), 10);
        assert_eq!(ids.len(), 10);
        assert_eq!((ids[0], ids[9]), (BOS, EOS));
    }

    #[test]
    fn falsches_format_wird_abgewiesen() {
        assert!(Zerleger::aus_json(br#"{"model":{"type":"BPE","vocab":[]}}"#).is_err());
        assert!(Zerleger::aus_json(b"kaputt").is_err());
    }
}
