//! `multilingual-e5-small` (MIT) über ONNX Runtime (`ort`, dieselbe gepinnte Version wie die
//! Spracherkennung). Läuft nur lokal; Vektoren verlassen den Prozess nur in Richtung `ablage/index.db`.
//!
//! Vorgaben des Modells: Anfragen mit `query: `, Stücke mit `passage: `, Mittelwert der letzten
//! verborgenen Zustände über die Aufmerksamkeitsmaske, L2-normiert (384 Dimensionen).

use super::zerleger::{Zerleger, PAD};
use std::path::Path;
use std::sync::Mutex;

/// Höchstlänge in Zerleger-Einheiten (Stücke der Ablage sind ≤ 220 Zeichen, Anfragen ≤ 200).
const MAX_EINHEITEN: usize = 256;
/// Stapelgröße beim Einbetten vieler Stücke.
const STAPEL: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    Anfrage,
    Stueck,
}

impl Art {
    fn praefix(self) -> &'static str {
        match self {
            Art::Anfrage => "query: ",
            Art::Stueck => "passage: ",
        }
    }
}

pub struct E5 {
    sitzung: Mutex<ort::session::Session>,
    zerleger: Zerleger,
    mit_typen: bool,
}

fn fehler(_e: impl std::fmt::Display) -> String {
    // Keine Inhalte und keine Pfade in Fehlertexten.
    "semantik.modell".to_string()
}

impl E5 {
    /// Lädt `model.onnx` und `tokenizer.json` aus `ordner`.
    pub fn laden(onnx: &Path, zerleger_json: &Path) -> Result<Self, String> {
        use ort::session::builder::GraphOptimizationLevel;
        if !onnx.is_file() || !zerleger_json.is_file() {
            return Err("semantik.fehlt".into());
        }
        let zerleger =
            Zerleger::aus_json(&std::fs::read(zerleger_json).map_err(fehler)?).map_err(fehler)?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().clamp(1, 4))
            .unwrap_or(2);
        let sitzung = ort::session::Session::builder()
            .map_err(fehler)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(fehler)?
            .with_intra_threads(threads)
            .map_err(fehler)?
            .with_inter_threads(1)
            .map_err(fehler)?
            .commit_from_file(onnx)
            .map_err(fehler)?;
        let mit_typen = sitzung
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        Ok(Self {
            sitzung: Mutex::new(sitzung),
            zerleger,
            mit_typen,
        })
    }

    /// Vektoren (L2-normiert) für `texte`, in Stapeln.
    pub fn einbetten(&self, texte: &[String], art: Art) -> Result<Vec<Vec<f32>>, String> {
        let mut aus = Vec::with_capacity(texte.len());
        for stapel in texte.chunks(STAPEL) {
            aus.extend(self.stapel(stapel, art)?);
        }
        Ok(aus)
    }

    fn stapel(&self, texte: &[String], art: Art) -> Result<Vec<Vec<f32>>, String> {
        if texte.is_empty() {
            return Ok(vec![]);
        }
        let ids: Vec<Vec<i64>> = texte
            .iter()
            .map(|t| {
                self.zerleger
                    .kodieren(&format!("{}{t}", art.praefix()), MAX_EINHEITEN)
            })
            .collect();
        let b = ids.len();
        let n = ids.iter().map(Vec::len).max().unwrap_or(1);
        let mut eingabe = vec![PAD; b * n];
        let mut maske = vec![0i64; b * n];
        for (z, zeile) in ids.iter().enumerate() {
            eingabe[z * n..z * n + zeile.len()].copy_from_slice(zeile);
            maske[z * n..z * n + zeile.len()].fill(1);
        }
        let t_ids = ort::value::Tensor::from_array(([b, n], eingabe)).map_err(fehler)?;
        let t_maske = ort::value::Tensor::from_array(([b, n], maske.clone())).map_err(fehler)?;
        let mut sitzung = self.sitzung.lock().unwrap_or_else(|e| e.into_inner());
        let ausgaben = if self.mit_typen {
            let t_typ =
                ort::value::Tensor::from_array(([b, n], vec![0i64; b * n])).map_err(fehler)?;
            sitzung
                .run(ort::inputs![
                    "input_ids" => t_ids,
                    "attention_mask" => t_maske,
                    "token_type_ids" => t_typ
                ])
                .map_err(fehler)?
        } else {
            sitzung
                .run(ort::inputs!["input_ids" => t_ids, "attention_mask" => t_maske])
                .map_err(fehler)?
        };
        let (form, daten) = ausgaben[0].try_extract_tensor::<f32>().map_err(fehler)?;
        let dim = ausgabe_pruefen(form, daten.len(), b, n)?;
        (0..b)
            .map(|z| {
                mittel(
                    &daten[z * n * dim..(z + 1) * n * dim],
                    &maske[z * n..(z + 1) * n],
                    dim,
                )
                .ok_or_else(|| "semantik.modell".to_string())
            })
            .collect()
    }
}

/// Prüft Form und Größe der Modellausgabe `[b, n, DIMENSION]` gegen die Eingabe und liefert die
/// Dimension. Ein anderes Modell (andere Breite, abgeschnittene Daten, negative oder übergroße Maße)
/// ergäbe sonst Vektoren falscher Länge im Index oder einen Indexfehler beim Schneiden.
fn ausgabe_pruefen(form: &[i64], laenge: usize, b: usize, n: usize) -> Result<usize, String> {
    let fehler = || "semantik.modell".to_string();
    let [fb, fn_, fd] = form else {
        return Err(fehler());
    };
    let (fb, fn_, fd) = (
        usize::try_from(*fb).map_err(|_| fehler())?,
        usize::try_from(*fn_).map_err(|_| fehler())?,
        usize::try_from(*fd).map_err(|_| fehler())?,
    );
    if fb != b || fn_ != n || fd != super::DIMENSION {
        return Err(fehler());
    }
    let erwartet = b
        .checked_mul(n)
        .and_then(|x| x.checked_mul(fd))
        .ok_or_else(fehler)?;
    if laenge != erwartet {
        return Err(fehler());
    }
    Ok(fd)
}

/// Mittelwert über die maskierten Zeilen, L2-normiert. `None`, wenn das Ergebnis nicht endlich ist.
fn mittel(zustaende: &[f32], maske: &[i64], dim: usize) -> Option<Vec<f32>> {
    let mut v = vec![0f32; dim];
    let mut zahl = 0f32;
    for (i, m) in maske.iter().enumerate() {
        if *m == 0 {
            continue;
        }
        zahl += 1.0;
        for (a, x) in v.iter_mut().zip(&zustaende[i * dim..(i + 1) * dim]) {
            *a += x;
        }
    }
    if zahl > 0.0 {
        v.iter_mut().for_each(|a| *a /= zahl);
    }
    super::normieren(&mut v).then_some(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mittel_beachtet_die_maske_und_normiert() {
        let z = [1.0, 0.0, 3.0, 0.0, 100.0, 100.0];
        let v = mittel(&z, &[1, 1, 0], 2).unwrap();
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1].abs() < 1e-6, "{v:?}");
    }

    #[test]
    fn mittel_weist_nicht_endliche_werte_ab() {
        assert!(mittel(&[f32::NAN, 1.0], &[1], 2).is_none());
        assert!(mittel(&[f32::INFINITY, 1.0], &[1], 2).is_none());
        // Zwei endliche Werte, deren Summe in f32 überläuft.
        assert!(mittel(&[f32::MAX, 0.0, f32::MAX, 0.0], &[1, 1], 2).is_none());
    }

    #[test]
    fn ausgabeform_muss_zu_eingabe_und_384_passen() {
        let ok = |form: &[i64], len: usize| ausgabe_pruefen(form, len, 2, 5);
        assert_eq!(ok(&[2, 5, 384], 2 * 5 * 384), Ok(384));
        // Andere Dimension (anderes Modell).
        assert!(ok(&[2, 5, 768], 2 * 5 * 768).is_err());
        // Falsche Stapel- oder Längenangabe, Rang, Datenlänge.
        assert!(ok(&[1, 5, 384], 5 * 384).is_err());
        assert!(ok(&[2, 4, 384], 2 * 4 * 384).is_err());
        assert!(ok(&[2, 5], 10).is_err());
        assert!(ok(&[2, 5, 384], 2 * 5 * 384 - 1).is_err());
        // Negative Maße wären als `usize` riesig.
        assert!(ok(&[2, 5, -1], 0).is_err());
        assert!(ok(&[-2, 5, 384], 0).is_err());
    }

    #[test]
    fn fehlende_dateien_werden_gemeldet() {
        let d = tempfile::tempdir().unwrap();
        let e = E5::laden(&d.path().join("model.onnx"), &d.path().join("t.json"));
        assert_eq!(e.err().as_deref(), Some("semantik.fehlt"));
    }
}
