//! Die Sprach-Engines hinter dem Sprachdienst (ADR-0021, Paket S2).
//!
//! Der Dienst kennt die Engines der App nur über [`Sprache`]. Die App-Anbindung steht in
//! `app.rs` (Erkennung über den `TranscriptionManager`, Stimmen über den `TtsManager`); Tests
//! benutzen Attrappen. Alle Aufrufe blockieren und laufen im Dienst auf eigenen Threads
//! (`spawn_blocking`), nie auf der Laufzeit der Verbindungen.
//!
//! Keine Inhalte im Log: weder Audio noch Text. Fehler tragen feste Meldungen, nie Auszüge
//! aus dem Gesprochenen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use wai_voice_dienst::msg::{code, Engines};

/// Fehler einer Engine: ein stabiler Code des Vertrags und eine feste, inhaltsfreie Meldung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineFehler {
    pub code: &'static str,
    pub nachricht: String,
}

impl EngineFehler {
    /// Es gibt keine Engine (kein Modell gewählt, Stimme nicht installiert).
    pub fn fehlt(nachricht: impl Into<String>) -> Self {
        Self {
            code: code::ENGINE_FEHLT,
            nachricht: nachricht.into(),
        }
    }

    /// Der Auftrag wurde abgebrochen (Abbruch der App, Widerruf, Verbindungsende).
    pub fn abgebrochen() -> Self {
        Self {
            code: code::ABGEBROCHEN,
            nachricht: "Der Auftrag wurde abgebrochen.".to_string(),
        }
    }

    /// Die Engine lief, lieferte aber nichts.
    pub fn fehlgeschlagen(nachricht: impl Into<String>) -> Self {
        Self {
            code: code::FEHLGESCHLAGEN,
            nachricht: nachricht.into(),
        }
    }
}

/// Hören und Sprechen, ohne Denken (`SpeechIO` in `@wai/voice`).
pub trait Sprache: Send + Sync + 'static {
    /// Namen der laufenden Engines und ob beide auf diesem Rechner laufen. Die Souveränität
    /// `nur_lokal` verlangt `lokal == true`, sonst antwortet der Dienst mit `nicht_lokal`.
    fn engines(&self) -> Engines;

    /// Ist ein Erkennungsmodell gewählt, kann der Dienst also Aufträge zum Erkennen annehmen
    /// (lva#610)? Das sagt nichts darüber, ob das Modell gerade geladen ist: Ein entladenes
    /// Modell wird beim Auftrag nachgeladen. Quelle für `Status.bereit`.
    fn bereit(&self) -> bool {
        true
    }

    /// Kann der Dienst jetzt Aufträge zum Sprechen annehmen (Sprachausgabe vorhanden)? Das
    /// Sprechen hängt nicht am Erkennungsmodell (lva#610).
    fn sprechen_bereit(&self) -> bool {
        true
    }

    /// Erkennt eine Äußerung: PCM16 LE, 16 kHz, mono. Blockiert. `abbruch` setzen Abbruch der App,
    /// Widerruf und das Ende der Verbindung; die Engine prüft es vor und zwischen ihren
    /// Teilschritten (ein laufender Erkennungsdurchgang selbst lässt sich nicht unterbrechen).
    fn erkennen(
        &self,
        pcm: &[u8],
        bereich: &str,
        abbruch: &Arc<AtomicBool>,
    ) -> Result<String, EngineFehler>;

    /// Spricht `text`; Rückgabe: vollständige WAV-Stücke in Sprechreihenfolge (je Satz eines).
    /// Blockiert, schreibt keine Datei, spielt nichts ab und benutzt keinen Cache. `abbruch` wie
    /// bei `erkennen`.
    fn sprechen(
        &self,
        text: &str,
        stimme: Option<&str>,
        abbruch: &Arc<AtomicBool>,
    ) -> Result<Vec<Vec<u8>>, EngineFehler>;

    /// Korrektursignal (Recht `lernen`). Local Voice AI nimmt es in Paket S2 noch nicht an; der
    /// Dienst vergibt das Recht deshalb nicht.
    fn rueckmeldung(&self, _bereich: &str, _paare: usize) -> Result<(), EngineFehler> {
        Err(EngineFehler::fehlt(
            "Lernen ist in dieser Version nicht verfügbar.",
        ))
    }
}

/// PCM16 LE nach f32 im Bereich -1,0 bis 1,0 (das Format der Erkennung der App). Ein
/// übriges ungerades Byte wird verworfen.
pub fn pcm_zu_f32(pcm: &[u8]) -> Vec<f32> {
    pcm.chunks_exact(2)
        .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32768.0)
        .collect()
}

/// Das Erkennungsmodell der App, so weit der Dienst es braucht, um es bei Bedarf nachzuladen
/// (lva#610). Die App-Anbindung steht in `app.rs`; der Prüfstand benutzt eine Attrappe.
pub trait Modellzugang {
    /// Ist in den Einstellungen ein Erkennungsmodell gewählt (auch wenn es gerade entladen ist)?
    fn gewaehlt(&self) -> bool;
    /// Ist das gewählte Modell geladen?
    fn geladen(&self) -> bool;
    /// Läuft gerade ein Ladevorgang? Muss nach `laden_anstossen` ohne Verzug `true` sein, bis der
    /// Vorgang endet.
    fn laedt(&self) -> bool;
    /// Stößt das Laden im Hintergrund an; kehrt sofort zurück.
    fn laden_anstossen(&self);
}

/// Stellt sicher, dass das gewählte Erkennungsmodell geladen ist, und lädt es bei Bedarf nach.
///
/// - Kein Modell gewählt: `engine_fehlt`.
/// - Endet der Ladevorgang, ohne dass ein Modell geladen ist, ist der Auftrag sofort
///   fehlgeschlagen; es wird nicht bis zur Wartefrist weitergewartet.
/// - Das Abbruch-Flag wird in jedem Takt geprüft (`abgebrochen`).
/// - Nach `warten` ohne Ergebnis: `fehlgeschlagen`.
///
/// Keine Inhalte im Fehler: nur feste Meldungen.
pub fn modell_bereitstellen(
    zugang: &dyn Modellzugang,
    abbruch: &Arc<AtomicBool>,
    warten: Duration,
    takt: Duration,
) -> Result<(), EngineFehler> {
    if abbruch.load(Ordering::Acquire) {
        return Err(EngineFehler::abgebrochen());
    }
    if !zugang.gewaehlt() {
        return Err(EngineFehler::fehlt("Es ist kein Erkennungsmodell gewählt."));
    }
    if zugang.geladen() {
        return Ok(());
    }
    zugang.laden_anstossen();
    let bis = Instant::now() + warten;
    loop {
        if zugang.geladen() {
            return Ok(());
        }
        if abbruch.load(Ordering::Acquire) {
            return Err(EngineFehler::abgebrochen());
        }
        if !zugang.laedt() {
            // Der Vorgang kann zwischen den beiden Abfragen geendet haben.
            if zugang.geladen() {
                return Ok(());
            }
            return Err(EngineFehler::fehlgeschlagen(
                "Das Erkennungsmodell konnte nicht geladen werden.",
            ));
        }
        if Instant::now() >= bis {
            return Err(EngineFehler::fehlgeschlagen(
                "Das Erkennungsmodell wurde nicht rechtzeitig geladen.",
            ));
        }
        std::thread::sleep(takt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_wird_normiert() {
        let pcm = [0x00, 0x00, 0x00, 0x40, 0x00, 0x80, 0xFF];
        let f = pcm_zu_f32(&pcm);
        assert_eq!(f.len(), 3, "das übrige Byte entfällt");
        assert_eq!(f[0], 0.0);
        assert_eq!(f[1], 0.5);
        assert_eq!(f[2], -1.0);
    }

    use std::sync::atomic::AtomicUsize;
    use wai_voice_dienst::msg::code;

    /// Was der Ladevorgang der Attrappe tut, sobald er angestoßen ist.
    #[derive(Clone, Copy)]
    enum Laden {
        /// Nach kurzer Zeit ist das Modell geladen.
        Erfolg,
        /// Nach kurzer Zeit endet der Ladevorgang ohne Modell.
        Fehlschlag,
        /// Der Ladevorgang endet nie.
        Haengt,
    }

    struct Modell {
        gewaehlt: bool,
        geladen: AtomicBool,
        laedt: AtomicBool,
        anstoesse: AtomicUsize,
        laden: Laden,
    }

    impl Modell {
        fn neu(gewaehlt: bool, geladen: bool, laden: Laden) -> Arc<Self> {
            Arc::new(Self {
                gewaehlt,
                geladen: AtomicBool::new(geladen),
                laedt: AtomicBool::new(false),
                anstoesse: AtomicUsize::new(0),
                laden,
            })
        }
    }

    struct Zugang(Arc<Modell>);

    impl Modellzugang for Zugang {
        fn gewaehlt(&self) -> bool {
            self.0.gewaehlt
        }
        fn geladen(&self) -> bool {
            self.0.geladen.load(Ordering::SeqCst)
        }
        fn laedt(&self) -> bool {
            self.0.laedt.load(Ordering::SeqCst)
        }
        fn laden_anstossen(&self) {
            self.0.anstoesse.fetch_add(1, Ordering::SeqCst);
            // Wie in der App: Das Flag steht, bevor der Anstoß zurückkehrt.
            self.0.laedt.store(true, Ordering::SeqCst);
            let m = self.0.clone();
            std::thread::spawn(move || match m.laden {
                Laden::Erfolg => {
                    std::thread::sleep(Duration::from_millis(40));
                    m.geladen.store(true, Ordering::SeqCst);
                    m.laedt.store(false, Ordering::SeqCst);
                }
                Laden::Fehlschlag => {
                    std::thread::sleep(Duration::from_millis(40));
                    m.laedt.store(false, Ordering::SeqCst);
                }
                Laden::Haengt => {}
            });
        }
    }

    const TAKT: Duration = Duration::from_millis(5);

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn ein_entladenes_gewaehltes_modell_wird_nachgeladen() {
        let m = Modell::neu(true, false, Laden::Erfolg);
        let r = modell_bereitstellen(&Zugang(m.clone()), &flag(), Duration::from_secs(30), TAKT);
        assert_eq!(r, Ok(()));
        assert_eq!(m.anstoesse.load(Ordering::SeqCst), 1, "genau ein Anstoß");
        assert!(m.geladen.load(Ordering::SeqCst));
    }

    #[test]
    fn ein_geladenes_modell_wird_nicht_nachgeladen() {
        let m = Modell::neu(true, true, Laden::Erfolg);
        let r = modell_bereitstellen(&Zugang(m.clone()), &flag(), Duration::from_secs(30), TAKT);
        assert_eq!(r, Ok(()));
        assert_eq!(m.anstoesse.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ohne_gewaehltes_modell_kommt_engine_fehlt() {
        let m = Modell::neu(false, false, Laden::Erfolg);
        let r = modell_bereitstellen(&Zugang(m.clone()), &flag(), Duration::from_secs(30), TAKT);
        assert_eq!(r.unwrap_err().code, code::ENGINE_FEHLT);
        assert_eq!(m.anstoesse.load(Ordering::SeqCst), 0, "nichts angestoßen");
    }

    #[test]
    fn ein_ladefehler_ergibt_fehlgeschlagen_ohne_wartefrist() {
        let m = Modell::neu(true, false, Laden::Fehlschlag);
        let beginn = Instant::now();
        let r = modell_bereitstellen(&Zugang(m), &flag(), Duration::from_secs(60), TAKT);
        assert_eq!(r.unwrap_err().code, code::FEHLGESCHLAGEN);
        assert!(
            beginn.elapsed() < Duration::from_secs(5),
            "der Fehler kommt, sobald der Ladevorgang endet, nicht erst nach der Wartefrist"
        );
    }

    #[test]
    fn ein_abbruch_waehrend_des_ladens_ergibt_abgebrochen() {
        let m = Modell::neu(true, false, Laden::Haengt);
        let abbruch = flag();
        let a = abbruch.clone();
        let setzer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(40));
            a.store(true, Ordering::Release);
        });
        let beginn = Instant::now();
        let r = modell_bereitstellen(&Zugang(m), &abbruch, Duration::from_secs(60), TAKT);
        setzer.join().unwrap();
        assert_eq!(r.unwrap_err().code, code::ABGEBROCHEN);
        assert!(beginn.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn ein_schon_gesetzter_abbruch_laedt_gar_nicht_erst() {
        let m = Modell::neu(true, false, Laden::Erfolg);
        let abbruch = Arc::new(AtomicBool::new(true));
        let r = modell_bereitstellen(&Zugang(m.clone()), &abbruch, Duration::from_secs(30), TAKT);
        assert_eq!(r.unwrap_err().code, code::ABGEBROCHEN);
        assert_eq!(m.anstoesse.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ein_haengender_ladevorgang_endet_nach_der_wartefrist() {
        let m = Modell::neu(true, false, Laden::Haengt);
        let r = modell_bereitstellen(&Zugang(m), &flag(), Duration::from_millis(120), TAKT);
        assert_eq!(r.unwrap_err().code, code::FEHLGESCHLAGEN);
    }
}
