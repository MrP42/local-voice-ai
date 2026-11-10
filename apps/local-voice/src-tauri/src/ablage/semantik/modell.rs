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
        if form.len() != 3 || form[0] as usize != b || form[1] as usize != n {
            return Err("semantik.modell".into());
        }
        let dim = form[2] as usize;
        Ok((0..b)
            .map(|z| {
                mittel(
                    &daten[z * n * dim..(z + 1) * n * dim],
                    &maske[z * n..(z + 1) * n],
                    dim,
                )
            })
            .collect())
    }
}

/// Mittelwert über die maskierten Zeilen, L2-normiert.
fn mittel(zustaende: &[f32], maske: &[i64], dim: usize) -> Vec<f32> {
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
    super::normieren(&mut v);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mittel_beachtet_die_maske_und_normiert() {
        let z = [1.0, 0.0, 3.0, 0.0, 100.0, 100.0];
        let v = mittel(&z, &[1, 1, 0], 2);
        assert!((v[0] - 1.0).abs() < 1e-6 && v[1].abs() < 1e-6, "{v:?}");
    }

    #[test]
    fn fehlende_dateien_werden_gemeldet() {
        let d = tempfile::tempdir().unwrap();
        let e = E5::laden(&d.path().join("model.onnx"), &d.path().join("t.json"));
        assert_eq!(e.err().as_deref(), Some("semantik.fehlt"));
    }
}
