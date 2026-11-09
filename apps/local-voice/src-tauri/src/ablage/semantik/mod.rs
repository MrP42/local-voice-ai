//! D6: Semantische Suche der Ablage (Spec §7). `multilingual-e5-small` (MIT, ONNX) erzeugt lokal Vektoren
//! für die Stücke von `ablage/index.db`; `wai-retrieval` mischt BM25 und Kosinus per RRF.
//!
//! * Ohne Modell oder ausgeschaltet: [`AblageEinbettung::Keine`] (Kennung `keiner`, wie D2) – die Suche
//!   bleibt rein lexikalisch.
//! * Kandidatenschwelle: `wai-retrieval` lässt Vektortreffer ab der festen Grenze
//!   [`wai_retrieval::MIN_COSINE`] zu. e5 liegt auch bei fremden Texten um 0,8; deshalb werden die Vektoren
//!   verschoben: Anfrage `[a·q, b]`, Stück `[a·p, −b]`. Das Skalarprodukt ist `a²·cos − b²` und erreicht
//!   die Grenze genau bei `cos = schwelle` (`a² = (1 + MIN_COSINE)/(1 + schwelle)`). Die Reihenfolge bleibt
//!   gleich. `wai-retrieval` ruft `embed` nur für Anfragen und `embed_batch` nur für Stücke auf.
//! * Vektoren liegen nur in `ablage/index.db` (abgeleitet, nie exportiert, nie synchronisiert). Freigabe-,
//!   Datenklassen- und Papierkorbfilter wirken in SQL vor dem Ranking, für beide Listen.
//! * Ob die semantische Suche Standard ist, entscheidet das Eval-Tor ([`SEMANTIK_STANDARD`], Test
//!   `tests::eval_tor_mit_echtem_modell`).

pub mod modell;
pub mod zerleger;

#[cfg(test)]
mod tests;

use crate::catalog::{Purpose, TtsCatalogEntry};
use modell::{Art, E5};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use wai_retrieval::{Embedder, FakeEmbedder, RetrievalError};

/// Katalog-ID des Modells.
pub const MODELL_ID: &str = "ablage-e5-small";
/// Unterordner unter `<Datenordner>/models` (nicht oben: der Modell-Scan hielte es sonst für ASR).
pub const MODELL_ORDNER: &str = "ablage-einbettung";
pub const DATEI_MODELL: &str = "model.onnx";
pub const DATEI_ZERLEGER: &str = "tokenizer.json";
/// Kosinus, ab dem ein Stück als Vektorkandidat gilt (am Eval-Korpus gewählt).
pub const KANDIDAT_COS: f32 = 0.83;
/// Ergebnis des Eval-Tors (09.11.2026, e5-small fp32): Recall@5 und Belegkorrektheit verbessert, aber
/// unbeantwortbare Fragen nur 0,35 „kein Beleg“ (Tor 0,9). Deshalb bleibt lexikalisch Standard und die
/// semantische Suche ist ein Opt-in.
pub const SEMANTIK_STANDARD: bool = false;

/// L2-Normierung an Ort und Stelle (Nullvektor bleibt).
pub fn normieren(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Katalogeintrag des Modells (zwei Dateien mit Größe und SHA-256).
pub fn katalog() -> Option<TtsCatalogEntry> {
    crate::catalog::tts_entries(Purpose::AblageEmbedding)
        .into_iter()
        .find(|e| e.id == MODELL_ID)
}

/// Ordner des Modells unter `models_root` (`<Datenordner>/models`).
pub fn modell_ordner(models_root: &Path) -> PathBuf {
    models_root.join(MODELL_ORDNER)
}

/// Sind beide Dateien vorhanden?
pub fn modell_bereit(models_root: &Path) -> bool {
    let d = modell_ordner(models_root);
    d.join(DATEI_MODELL).is_file() && d.join(DATEI_ZERLEGER).is_file()
}

/// Rechenkern hinter der Einbettung.
#[derive(Clone)]
pub enum Kern {
    E5(Arc<E5>),
    /// Deterministischer Hash-Behelf aus `wai-retrieval` (Tests; keine Semantik).
    #[cfg_attr(not(test), allow(dead_code))]
    Test(Arc<FakeEmbedder>),
}

impl Kern {
    fn id(&self) -> String {
        match self {
            // Revision des Katalogs: ein anderes Modell ergibt eine andere Kennung (Neuaufbau).
            Kern::E5(_) => "e5-small@614241f6".into(),
            Kern::Test(f) => f.id(),
        }
    }

    async fn vektoren(&self, texte: &[String], art: Art) -> wai_retrieval::Result<Vec<Vec<f32>>> {
        match self {
            Kern::E5(m) => m.einbetten(texte, art).map_err(RetrievalError::Embedding),
            Kern::Test(f) => f.embed_batch(texte).await,
        }
    }
}

/// Einbettung der Ablage: keine (lexikalisch) oder Vektoren mit Kandidatenschwelle.
#[derive(Clone)]
pub enum AblageEinbettung {
    Keine,
    Vektor { kern: Kern, schwelle: f32 },
}

impl AblageEinbettung {
    pub fn ist_semantisch(&self) -> bool {
        matches!(self, Self::Vektor { .. })
    }

    /// Hash-Behelf für Tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn test(schwelle: f32) -> Self {
        Self::Vektor {
            kern: Kern::Test(Arc::new(FakeEmbedder::new(1024))),
            schwelle,
        }
    }

    pub fn e5(m: Arc<E5>) -> Self {
        Self::Vektor {
            kern: Kern::E5(m),
            schwelle: KANDIDAT_COS,
        }
    }
}

/// Verschiebt einen normierten Vektor (siehe Moduldoku): Anfrage `+b`, Stück `−b`.
pub fn verschieben(mut v: Vec<f32>, schwelle: f32, art: Art) -> Vec<f32> {
    let a2 = (1.0 + wai_retrieval::MIN_COSINE) / (1.0 + schwelle.clamp(-0.99, 0.99));
    let a2 = a2.clamp(0.0, 1.0);
    let (a, b) = (a2.sqrt(), (1.0 - a2).sqrt());
    v.iter_mut().for_each(|x| *x *= a);
    v.push(match art {
        Art::Anfrage => b,
        Art::Stueck => -b,
    });
    v
}

impl Embedder for AblageEinbettung {
    fn id(&self) -> String {
        match self {
            // Gleiche Kennung wie D2: ein bestehender lexikalischer Index bleibt gültig.
            Self::Keine => "keiner".into(),
            Self::Vektor { kern, schwelle } => format!("{}|kand={schwelle:.2}", kern.id()),
        }
    }

    async fn embed(&self, text: &str) -> wai_retrieval::Result<Vec<f32>> {
        match self {
            Self::Keine => Err(RetrievalError::Embedding("lexikalisch".into())),
            Self::Vektor { kern, schwelle } => {
                let v = kern
                    .vektoren(&[text.to_string()], Art::Anfrage)
                    .await?
                    .pop()
                    .ok_or_else(|| RetrievalError::Embedding("leer".into()))?;
                Ok(verschieben(v, *schwelle, Art::Anfrage))
            }
        }
    }

    async fn embed_batch(&self, texts: &[String]) -> wai_retrieval::Result<Vec<Vec<f32>>> {
        match self {
            Self::Keine => Err(RetrievalError::Embedding("lexikalisch".into())),
            Self::Vektor { kern, schwelle } => Ok(kern
                .vektoren(texts, Art::Stueck)
                .await?
                .into_iter()
                .map(|v| verschieben(v, *schwelle, Art::Stueck))
                .collect()),
        }
    }

    fn is_fallback(&self) -> bool {
        matches!(
            self,
            Self::Vektor {
                kern: Kern::Test(_),
                ..
            }
        )
    }
}

// ---------------------------------------------------------------------------
// Prozesszustand: Index-Abgleich und Suche laufen ohne AppHandle.
// ---------------------------------------------------------------------------

struct Zustand {
    models_root: Option<PathBuf>,
    an: bool,
    modell: Option<Arc<E5>>,
    /// Laden ist fehlgeschlagen (bis zum nächsten Einrichten/Laden nicht erneut versuchen).
    ladefehler: bool,
}

static ZUSTAND: Mutex<Zustand> = Mutex::new(Zustand {
    models_root: None,
    an: false,
    modell: None,
    ladefehler: false,
});

fn zustand() -> std::sync::MutexGuard<'static, Zustand> {
    ZUSTAND.lock().unwrap_or_else(|e| e.into_inner())
}

/// Beim Start und nach jeder Änderung: Modellordner und Schalter.
pub fn einrichten(models_root: &Path, an: bool) {
    let mut z = zustand();
    if z.models_root.as_deref() != Some(models_root) {
        z.modell = None;
    }
    z.models_root = Some(models_root.to_path_buf());
    z.an = an;
    z.ladefehler = false;
    if !an {
        // Speicher freigeben: das Modell braucht rund 500 MB.
        z.modell = None;
    }
}

/// Die wirksame Einbettung: Vektoren nur, wenn eingeschaltet und das Modell geladen werden kann.
pub fn aktuell() -> AblageEinbettung {
    let mut z = zustand();
    if !z.an || z.ladefehler {
        return AblageEinbettung::Keine;
    }
    if let Some(m) = &z.modell {
        return AblageEinbettung::e5(m.clone());
    }
    let Some(root) = z.models_root.clone() else {
        return AblageEinbettung::Keine;
    };
    if !modell_bereit(&root) {
        return AblageEinbettung::Keine;
    }
    let d = modell_ordner(&root);
    match E5::laden(&d.join(DATEI_MODELL), &d.join(DATEI_ZERLEGER)) {
        Ok(m) => {
            let m = Arc::new(m);
            z.modell = Some(m.clone());
            AblageEinbettung::e5(m)
        }
        Err(code) => {
            log::warn!("ablage: semantische Suche nicht verfügbar ({code})");
            z.ladefehler = true;
            AblageEinbettung::Keine
        }
    }
}

/// Entfernt Modell und Zerleger (samt halber Downloads) und gibt das geladene Modell frei. Während
/// Vektoren erstellt werden, wird nichts gelöscht (`semantik.belegt`). Vektoren im Suchindex bleiben
/// liegen (abgeleitet, lokal); ohne Modell wirkt nur die lexikalische Suche.
pub fn modell_entfernen(models_root: &Path) -> Result<(), String> {
    let mut z = zustand();
    if nachziehen_laeuft() {
        return Err("semantik.belegt".into());
    }
    z.modell = None;
    z.ladefehler = false;
    let d = modell_ordner(models_root);
    for name in [DATEI_MODELL, DATEI_ZERLEGER] {
        for p in [d.join(name), d.join(format!("{name}.partial"))] {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("semantik.ordner".into()),
            }
        }
    }
    // Nur ein leerer Ordner wird entfernt.
    let _ = std::fs::remove_dir(&d);
    Ok(())
}

/// Konnte das Modell nicht geladen werden?
pub fn ladefehler() -> bool {
    zustand().ladefehler
}

static NACHZIEHEN: AtomicBool = AtomicBool::new(false);

pub fn nachziehen_laeuft() -> bool {
    NACHZIEHEN.load(Ordering::Relaxed)
}

/// Bettet im Hintergrund fehlende Vektoren ein (Einschalten, Modellwechsel, Stücke aus der
/// lexikalischen Zeit). Höchstens ein Lauf zugleich; `fertig` meldet das Ende an die Oberfläche.
pub fn nachziehen_anstossen(dir: PathBuf, fertig: Box<dyn Fn() + Send>) {
    if !zustand().an || NACHZIEHEN.swap(true, Ordering::SeqCst) {
        return;
    }
    let gestartet = std::thread::Builder::new()
        .name("ablage-vektoren".into())
        .spawn(move || {
            struct Ende;
            impl Drop for Ende {
                fn drop(&mut self) {
                    NACHZIEHEN.store(false, Ordering::SeqCst);
                }
            }
            let _ende = Ende;
            let emb = aktuell();
            if emb.ist_semantisch() {
                let a = super::speicher::Ablage::neu(dir);
                if let Err(e) = super::index::vektoren_nachziehen(&a, &emb) {
                    log::warn!("ablage: Vektoren nachziehen: {}", e.code);
                }
            }
            drop(_ende);
            fertig();
        });
    if gestartet.is_err() {
        NACHZIEHEN.store(false, Ordering::SeqCst);
    }
}
