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

/// Ausgabedimension von `multilingual-e5-small`.
pub const DIMENSION: usize = 384;

/// L2-Normierung an Ort und Stelle (Nullvektor bleibt). `false` und unverändert, wenn ein Wert nicht
/// endlich ist (NaN, ±∞) oder die Norm überläuft: solche Vektoren dürfen nie in den Index (sie
/// vergifteten das Skalarprodukt aller Suchen). Die Norm läuft in `f64`, damit große endliche Werte nicht
/// in `f32` überlaufen.
pub fn normieren(v: &mut [f32]) -> bool {
    if v.iter().any(|x| !x.is_finite()) {
        return false;
    }
    let n = v
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        .sqrt();
    if n == 0.0 {
        return true;
    }
    let ergebnis: Vec<f32> = v.iter().map(|&x| (f64::from(x) / n) as f32).collect();
    if !ergebnis.iter().all(|x| x.is_finite()) {
        return false;
    }
    v.copy_from_slice(&ergebnis);
    true
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

/// Kennung des Modells aus der Revision des Katalogeintrags (`e5-small@<8 Zeichen>`). Eine neue
/// Revision im Katalog ergibt eine neue Kennung und damit neue Vektoren.
pub fn modell_kennung() -> &'static str {
    static KENNUNG: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KENNUNG.get_or_init(|| {
        let rev = katalog()
            .and_then(|e| e.revision)
            .unwrap_or_else(|| "unbekannt".into());
        format!("e5-small@{}", rev.chars().take(8).collect::<String>())
    })
}

/// Halbe Downloads im Modellordner (`*.partial`)?
pub fn reste_vorhanden(models_root: &Path) -> bool {
    let d = modell_ordner(models_root);
    [DATEI_MODELL, DATEI_ZERLEGER]
        .iter()
        .any(|n| d.join(format!("{n}.partial")).is_file())
}

/// Rechenkern hinter der Einbettung.
#[derive(Clone)]
pub enum Kern {
    E5(Arc<E5>),
    /// Deterministischer Hash-Behelf aus `wai-retrieval` (Tests; keine Semantik).
    #[cfg_attr(not(test), allow(dead_code))]
    /// Mit Pause je Stapel (Nebenläufigkeitstests).
    Test(Arc<FakeEmbedder>, std::time::Duration),
}

impl Kern {
    fn id(&self) -> String {
        match self {
            // Revision des Katalogs: ein anderes Modell ergibt eine andere Kennung (Neuaufbau).
            Kern::E5(_) => modell_kennung().to_string(),
            Kern::Test(f, _) => f.id(),
        }
    }

    async fn vektoren(&self, texte: &[String], art: Art) -> wai_retrieval::Result<Vec<Vec<f32>>> {
        match self {
            Kern::E5(m) => m.einbetten(texte, art).map_err(RetrievalError::Embedding),
            Kern::Test(f, pause) => {
                if !pause.is_zero() {
                    std::thread::sleep(*pause);
                }
                f.embed_batch(texte).await
            }
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
            kern: Kern::Test(Arc::new(FakeEmbedder::new(1024)), std::time::Duration::ZERO),
            schwelle,
        }
    }

    /// Hash-Behelf mit Pause je Stapel (simuliert ein langsames Modell).
    #[cfg(test)]
    pub fn test_langsam(schwelle: f32, pause: std::time::Duration) -> Self {
        Self::Vektor {
            kern: Kern::Test(Arc::new(FakeEmbedder::new(1024)), pause),
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
                kern: Kern::Test(..),
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

/// Serialisiert das Laden des Modells (nicht [`ZUSTAND`]: Einrichten und Stand warten nicht auf das
/// Laden von rund 500 MB).
static LADE_SPERRE: Mutex<()> = Mutex::new(());

/// Die wirksame Einbettung: Vektoren nur, wenn eingeschaltet und das Modell geladen werden kann. Das
/// Laden läuft außerhalb der Zustandssperre; wurde währenddessen umgeschaltet oder der Ordner
/// gewechselt, wird das Ergebnis verworfen.
pub fn aktuell() -> AblageEinbettung {
    let root = {
        let z = zustand();
        if !z.an || z.ladefehler {
            return AblageEinbettung::Keine;
        }
        if let Some(m) = &z.modell {
            return AblageEinbettung::e5(m.clone());
        }
        let Some(root) = z.models_root.clone() else {
            return AblageEinbettung::Keine;
        };
        root
    };
    let _laden = LADE_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
    // Ein anderer Aufrufer kann inzwischen geladen haben.
    {
        let z = zustand();
        if !z.an || z.ladefehler || z.models_root.as_deref() != Some(root.as_path()) {
            return AblageEinbettung::Keine;
        }
        if let Some(m) = &z.modell {
            return AblageEinbettung::e5(m.clone());
        }
    }
    if !modell_bereit(&root) {
        return AblageEinbettung::Keine;
    }
    let d = modell_ordner(&root);
    let geladen = E5::laden(&d.join(DATEI_MODELL), &d.join(DATEI_ZERLEGER));
    let mut z = zustand();
    if !z.an || z.models_root.as_deref() != Some(root.as_path()) {
        return AblageEinbettung::Keine;
    }
    match geladen {
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
    // Kein Laden parallel (es läse halb gelöschte Dateien).
    let _laden = LADE_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
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

/// Anstoß und Ende des Hintergrundlaufs laufen über genau einen Mutex: Wer entscheidet, dass der Worker
/// endet (keine Runde offen), gibt unter derselben Sperre frei. Ein Anstoß sieht damit entweder einen
/// laufenden Worker (und seine Runde wird von ihm noch gesehen) oder startet einen neuen Worker.
/// Jeder Worker hat eine eigene Kennung (Eigentümerschaft): Das spätere `Drop` eines beendeten Workers
/// gibt nur frei, solange er selbst noch Eigentümer ist, nie den Platz eines inzwischen gestarteten.
#[derive(Debug, Default)]
struct Anstoss {
    /// Kennung des laufenden Workers; `None`: keiner läuft.
    besitzer: Option<u64>,
    /// Kennung für den nächsten Worker.
    naechste_id: u64,
    /// Zahl aller Anstöße und die davon schon begonnenen Runden (Zähler statt Flag: prüfbar).
    angestossen: u64,
    begonnen: u64,
}

impl Anstoss {
    const fn neu() -> Self {
        Self {
            besitzer: None,
            naechste_id: 0,
            angestossen: 0,
            begonnen: 0,
        }
    }

    fn laeuft(&self) -> bool {
        self.besitzer.is_some()
    }

    /// Ein Anstoß; `Some(kennung)`, wenn der Aufrufer einen Worker mit dieser Kennung starten muss.
    fn anstossen(&mut self) -> Option<u64> {
        self.angestossen += 1;
        if self.besitzer.is_some() {
            return None;
        }
        self.naechste_id += 1;
        self.besitzer = Some(self.naechste_id);
        self.besitzer
    }

    /// Der Worker `id` fragt nach der nächsten Runde. `false`: nichts offen, der Worker endet (der
    /// Platz ist dann bereits freigegeben, in derselben Sperre).
    fn naechste_runde(&mut self, id: u64) -> bool {
        if self.begonnen == self.angestossen {
            self.freigeben(id);
            return false;
        }
        self.begonnen = self.angestossen;
        true
    }

    /// Gibt den Platz frei, aber nur für den Eigentümer; ein später Aufruf eines alten Workers ist
    /// wirkungslos, sobald ein neuer Worker läuft.
    fn freigeben(&mut self, id: u64) {
        if self.besitzer == Some(id) {
            self.besitzer = None;
        }
    }

    /// Weitere Runde nach einem Fehler (zählt wie ein Anstoß des Laufs selbst).
    fn wiederholen(&mut self) {
        self.angestossen += 1;
    }
}

/// Gibt beim Verlassen des Worker-Threads (auch bei Panik) den Platz frei, sofern er noch gehört.
struct Ende<'a> {
    anstoss: &'a Mutex<Anstoss>,
    id: u64,
}

impl Drop for Ende<'_> {
    fn drop(&mut self) {
        self.anstoss
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .freigeben(self.id);
    }
}

static ANSTOSS: Mutex<Anstoss> = Mutex::new(Anstoss::neu());

fn anstoss() -> std::sync::MutexGuard<'static, Anstoss> {
    ANSTOSS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Weitere Versuche nach einem Fehler (mit wachsender Pause), solange Vektoren fehlen.
const NACHZIEHEN_VERSUCHE: u32 = 3;

pub fn nachziehen_laeuft() -> bool {
    anstoss().laeuft()
}

/// Bettet im Hintergrund fehlende Vektoren ein (Einschalten, Modellwechsel, Stücke aus der
/// lexikalischen Zeit). Höchstens ein Lauf zugleich; ein Anstoß während eines Laufs führt zu einer
/// weiteren Runde. Nach einem Fehler wird begrenzt erneut versucht. `fertig` meldet das Ende an die
/// Oberfläche.
pub fn nachziehen_anstossen(dir: PathBuf, fertig: Box<dyn Fn() + Send>) {
    if !zustand().an {
        return;
    }
    let Some(id) = anstoss().anstossen() else {
        return;
    };
    let gestartet = std::thread::Builder::new()
        .name("ablage-vektoren".into())
        .spawn(move || {
            // Auch bei einer Panik im Lauf freigeben (sonst startete nie wieder ein Worker).
            let _ende = Ende {
                anstoss: &ANSTOSS,
                id,
            };
            let a = super::speicher::Ablage::neu(dir);
            let mut fehlversuche = 0u32;
            while anstoss().naechste_runde(id) {
                let emb = aktuell();
                if !emb.ist_semantisch() {
                    continue;
                }
                match super::index::vektoren_nachziehen(&a, &emb) {
                    Ok(_) => fehlversuche = 0,
                    Err(e) => {
                        log::warn!("ablage: Vektoren nachziehen: {}", e.code);
                        let fehlen = super::index::vektor_stand(&a, &emb)
                            .map(|(neu, n)| neu || n > 0)
                            .unwrap_or(false);
                        if fehlen && fehlversuche < NACHZIEHEN_VERSUCHE {
                            fehlversuche += 1;
                            std::thread::sleep(std::time::Duration::from_secs(
                                2u64 << fehlversuche,
                            ));
                            anstoss().wiederholen();
                        }
                    }
                }
            }
            drop(_ende);
            fertig();
        });
    if gestartet.is_err() {
        anstoss().freigeben(id);
    }
}

#[cfg(test)]
mod tests_anstoss {
    use super::{Anstoss, Ende};
    use std::sync::{Arc, Mutex};

    #[test]
    fn zustandsmaschine_einzeln() {
        let mut s = Anstoss::neu();
        let a = s.anstossen().expect("erster Anstoß startet den Worker");
        assert!(
            s.anstossen().is_none(),
            "zweiter sieht den laufenden Worker"
        );
        assert!(
            s.naechste_runde(a),
            "beide Anstöße gelten als eine offene Runde"
        );
        assert!(!s.naechste_runde(a));
        assert!(!s.laeuft(), "Ende gibt in derselben Sperre frei");
        assert!(
            s.anstossen().is_some(),
            "danach startet ein neuer Anstoß wieder einen Worker"
        );
    }

    /// Codex-Befund 4: A endet regulär, B startet, danach läuft das `Drop` von A. Es darf B nicht
    /// freigeben, sonst startet C einen zweiten Worker. Geprüft mit dem Produktions-`Drop` (`Ende`).
    #[test]
    fn drop_des_alten_workers_gibt_den_neuen_nicht_frei() {
        let m = Mutex::new(Anstoss::neu());
        let a = m.lock().unwrap().anstossen().expect("A startet");
        let ende_a = Ende { anstoss: &m, id: a };
        assert!(m.lock().unwrap().naechste_runde(a), "A holt seine Runde");
        assert!(!m.lock().unwrap().naechste_runde(a), "A endet");
        let b = m.lock().unwrap().anstossen().expect("B startet");
        drop(ende_a);
        assert!(m.lock().unwrap().laeuft(), "B läuft weiter");
        assert!(
            m.lock().unwrap().anstossen().is_none(),
            "C darf keinen zweiten Worker starten"
        );
        let ende_b = Ende { anstoss: &m, id: b };
        drop(ende_b);
        assert!(
            !m.lock().unwrap().laeuft(),
            "Drop des Eigentümers gibt frei (Panikpfad)"
        );
    }

    /// Nebenläufigkeit: viele Anstoßgeber gegen einen Worker, der Runden holt und nach jeder Runde
    /// kurz arbeitet. Unverlierbar: Sobald der letzte Worker beendet ist und alle Anstöße geschehen
    /// sind, ist keine Runde offen (`begonnen == angestossen`), und es lief nie mehr als ein Worker.
    #[test]
    fn kein_anstoss_geht_verloren_und_nie_zwei_worker() {
        for _ in 0..200 {
            let s = Arc::new(Mutex::new(Anstoss::neu()));
            let aktiv = Arc::new(std::sync::atomic::AtomicI32::new(0));
            let spitze = Arc::new(std::sync::atomic::AtomicI32::new(0));
            let worker = Arc::new(Mutex::new(Vec::<std::thread::JoinHandle<()>>::new()));
            let geber: Vec<_> = (0..4)
                .map(|_| {
                    let (s, aktiv, spitze, worker) =
                        (s.clone(), aktiv.clone(), spitze.clone(), worker.clone());
                    std::thread::spawn(move || {
                        for _ in 0..50 {
                            let Some(id) = s.lock().unwrap().anstossen() else {
                                continue;
                            };
                            let (s, aktiv, spitze) = (s.clone(), aktiv.clone(), spitze.clone());
                            let h = std::thread::spawn(move || {
                                use std::sync::atomic::Ordering::SeqCst;
                                let jetzt = aktiv.fetch_add(1, SeqCst) + 1;
                                spitze.fetch_max(jetzt, SeqCst);
                                // Zwischen `naechste_runde` und dem Ende liegt Arbeit; gleich danach wird
                                // abgemeldet, noch vor dem Ende-Entscheid durch den nächsten Aufruf.
                                loop {
                                    let weiter = {
                                        let mut g = s.lock().unwrap();
                                        let w = g.naechste_runde(id);
                                        if !w {
                                            aktiv.fetch_sub(1, SeqCst);
                                        }
                                        w
                                    };
                                    if !weiter {
                                        break;
                                    }
                                    std::thread::yield_now();
                                }
                            });
                            worker.lock().unwrap().push(h);
                        }
                    })
                })
                .collect();
            for g in geber {
                g.join().unwrap();
            }
            let handles: Vec<_> = worker.lock().unwrap().drain(..).collect();
            for h in handles {
                h.join().unwrap();
            }
            let g = s.lock().unwrap();
            assert_eq!(g.begonnen, g.angestossen, "verlorener Anstoß");
            assert!(!g.laeuft());
            assert_eq!(
                spitze.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "zwei Worker"
            );
        }
    }
}
