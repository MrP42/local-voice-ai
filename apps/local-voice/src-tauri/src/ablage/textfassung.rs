//! Warteschlange der Textfassungen (Spec §4.1): ein Hintergrundarbeiter, abbrechbar. Nach dem Ablegen
//! oder einer neuen Fassung steht die Datei auf `ausstehend`; der Arbeiter erzeugt die Textfassung im
//! Unterprozess, schreibt sie atomar nach `text/<dat>/<textrev>.json`, trägt sie in `texte` der Fassung
//! ein (`ok`, `kein_text` oder `fehler` mit Code) und gleicht den Index ab.
//!
//! Wiederanlauf: Es gibt keine eigene Warteschlangendatei. Offen ist, was im Stand fehlt
//! ([`Ablage::text_auftraege`]); nach einem Neustart läuft der Arbeiter einfach erneut. Ein Fehler am
//! Inhalt (ZIP-Bombe, Prüfsumme, Format) wird als `fehler` vermerkt und nicht wiederholt; ein
//! vorübergehender Fehler (Start, Schutz, Absturz, Zeit, Lesen; [`super::speicher::VORUEBERGEHEND`])
//! wird mit Zähler vermerkt und höchstens [`super::speicher::MAX_VERSUCHE`]-mal versucht, jeweils erst
//! im nächsten Prozess (Rückstellung statt sofortiger Wiederholung). Eine neue Extraktorversion ergibt
//! eine neue `textrev` und damit einen neuen Versuch. Nie Inhalt im Log, nur Zähler und Codes.
//!
//! Abbruch: [`abbrechen`] hält die Warteschlange an. Der laufende Lauf bricht ab, und erst ein
//! ausdrücklicher Anstoß ([`anstossen`], z. B. Import) hebt das Anhalten auf; das Neuladen der Seite
//! ([`wiederanlauf`]) startet einen angehaltenen Arbeiter nicht neu.

use super::extrakt::Textfassung;
use super::speicher::{text_wiederholbar, Ablage, TextAblage, TextAuftrag};
use super::textlauf::{self, Grenzen, Programm};
use super::zusammenfuehren::Text;
use super::Res;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};

/// Ergebnis eines Laufs (Tests, Log).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Lauf {
    pub ok: u32,
    pub kein_text: u32,
    pub fehler: u32,
    pub abgebrochen: bool,
}

fn text_aus(t: &Textfassung, blob: String, size: u64) -> Text {
    Text {
        textrev: t.textrev.clone(),
        blob,
        size,
        extraktor: t.extraktor.clone(),
        extraktor_version: t.extraktor_version.clone(),
        marken_version: t.marken_version,
        stand: if t.abschnitte.is_empty() {
            "kein_text".into()
        } else {
            "ok".into()
        },
        code: None,
        versuche: 0,
    }
}

fn fehler_text(auf: &TextAuftrag, code: &str) -> Text {
    let (extraktor, version) = super::extrakt::format_aus_typ(&auf.typ)
        .map(super::extrakt::extraktor)
        .unwrap_or(("unbekannt", "0"));
    Text {
        textrev: auf.textrev.clone(),
        blob: String::new(),
        size: 0,
        extraktor: extraktor.into(),
        extraktor_version: version.into(),
        marken_version: super::extrakt::MARKEN_VERSION,
        stand: "fehler".into(),
        code: Some(code.chars().take(40).collect()),
        versuche: auf.versuche.saturating_add(1),
    }
}

/// Ausgang eines Auftrags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ausgang {
    Eingetragen(Text),
    /// Ergebnis nicht speicherbar (Datenträger, Verknüpfung im Textordner): nichts eingetragen, der
    /// nächste Start versucht es erneut.
    Uebersprungen,
    /// Durch die Bedienung abgebrochen: nichts eingetragen.
    Abgebrochen,
    /// Datei inzwischen gelöscht oder schon erledigt: nichts geschrieben.
    Verworfen,
}

/// Ein Auftrag: Unterprozess, Ergebnis schreiben und eintragen (zusammen unter der Ablage-Sperre;
/// ist die Datei inzwischen gelöscht, bleibt nichts zurück).
pub fn bearbeiten(
    a: &Ablage,
    auf: &TextAuftrag,
    prog: &Programm,
    grenzen: Grenzen,
    abbrechen: &AtomicBool,
) -> Res<Ausgang> {
    let ergebnis = textlauf::ausfuehren(
        prog,
        a.dir(),
        &auf.pfad,
        &auf.sha256,
        &auf.typ,
        grenzen,
        abbrechen,
    );
    let (text, json) = match ergebnis {
        Ok(t) if t.textrev == auf.textrev => {
            let Ok(json) = serde_json::to_vec(&t) else {
                return Ok(Ausgang::Uebersprungen);
            };
            (text_aus(&t, String::new(), 0), Some(json))
        }
        Ok(_) => (fehler_text(auf, "extrakt.antwort"), None),
        Err(code) if code == "extrakt.abgebrochen" => return Ok(Ausgang::Abgebrochen),
        Err(code) => (fehler_text(auf, &code), None),
    };
    match a.text_ablegen(&auf.dat, &auf.fas, json.as_deref(), text)? {
        TextAblage::Eingetragen(t) => Ok(Ausgang::Eingetragen(t)),
        TextAblage::NichtSpeicherbar => Ok(Ausgang::Uebersprungen),
        TextAblage::Unveraendert | TextAblage::Verschwunden => Ok(Ausgang::Verworfen),
    }
}

/// Aufträge, die in diesem Prozess nicht mehr versucht werden: Ergebnis nicht speicherbar oder
/// vorübergehender Fehler (erst der nächste Start versucht sie erneut; sonst liefe der Arbeiter bei
/// jedem Neuladen der Seite wieder los).
static UEBERSPRUNGEN: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

fn zurueckstellen(auf: &TextAuftrag) {
    if let Ok(mut u) = UEBERSPRUNGEN.lock() {
        u.insert((auf.fas.clone(), auf.textrev.clone()));
    }
}

fn uebersprungen() -> BTreeSet<(String, String)> {
    UEBERSPRUNGEN.lock().map(|u| u.clone()).unwrap_or_default()
}

/// Test: neuer Prozess (Rückstellungen dieser Fassung vergessen).
#[cfg(test)]
pub fn rueckstellung_vergessen(fas: &str) {
    if let Ok(mut u) = UEBERSPRUNGEN.lock() {
        u.retain(|(f, _)| f != fas);
    }
}

/// Gibt es Aufträge, die dieser Prozess noch bearbeiten würde (ohne Rückstellungen)?
pub fn offen(a: &Ablage) -> bool {
    let zurueck = uebersprungen();
    a.text_auftraege()
        .map(|v| {
            v.iter()
                .any(|x| !zurueck.contains(&(x.fas.clone(), x.textrev.clone())))
        })
        .unwrap_or(false)
}

/// Arbeitet alle offenen Aufträge ab und gleicht danach den Index ab. `melden` wird nach jedem
/// eingetragenen Ergebnis gerufen (Oberfläche neu laden).
pub fn lauf(
    a: &Ablage,
    prog: &Programm,
    grenzen: Grenzen,
    abbrechen: &AtomicBool,
    melden: &dyn Fn(),
) -> Res<Lauf> {
    let mut l = Lauf::default();
    let mut gesehen: BTreeSet<(String, String)> = BTreeSet::new();
    loop {
        let zurueck = uebersprungen();
        let auftraege: Vec<TextAuftrag> = a
            .text_auftraege()?
            .into_iter()
            .filter(|x| {
                let k = (x.fas.clone(), x.textrev.clone());
                !gesehen.contains(&k) && !zurueck.contains(&k)
            })
            .collect();
        if auftraege.is_empty() {
            break;
        }
        for auf in auftraege {
            if abbrechen.load(Ordering::Relaxed) {
                l.abgebrochen = true;
                let _ = super::index::abgleichen(a);
                return Ok(l);
            }
            gesehen.insert((auf.fas.clone(), auf.textrev.clone()));
            match bearbeiten(a, &auf, prog, grenzen, abbrechen)? {
                Ausgang::Eingetragen(t) if t.stand == "ok" => l.ok += 1,
                Ausgang::Eingetragen(t) if t.stand == "kein_text" => l.kein_text += 1,
                Ausgang::Eingetragen(t) => {
                    l.fehler += 1;
                    if text_wiederholbar(&t) {
                        // Vorübergehend: erst im nächsten Prozess wieder (Rückstellung).
                        zurueckstellen(&auf);
                    }
                }
                Ausgang::Uebersprungen => {
                    // Nichts geändert: kein Ereignis, und in diesem Prozess nicht erneut versuchen.
                    l.fehler += 1;
                    zurueckstellen(&auf);
                    continue;
                }
                Ausgang::Verworfen => continue,
                Ausgang::Abgebrochen => {
                    l.abgebrochen = true;
                    let _ = super::index::abgleichen(a);
                    return Ok(l);
                }
            }
            // Den Index gleich nachziehen: Treffer erscheinen, ohne auf die ganze Warteschlange zu warten.
            let _ = super::index::abgleichen(a);
            melden();
        }
    }
    super::index::abgleichen(a)?;
    Ok(l)
}

// ---------------------------------------------------------------------------
// Der eine Hintergrundarbeiter der App
// ---------------------------------------------------------------------------

/// Zustand der Warteschlange (ohne Thread, testbar). Alle Übergänge unter einer Sperre:
///
/// * [`Steuerung::anstossen`] (ausdrücklich: Import, Änderung durch die Bedienung) hebt das Anhalten
///   auf und reiht ein, setzt aber das Abbruchkennzeichen eines laufenden Laufs **nicht** zurück;
/// * [`Steuerung::wiederanlauf`] (Neuladen der Seite) reiht nur ein, wenn nicht angehalten;
/// * [`Steuerung::abbrechen`] setzt das Kennzeichen, leert die Schlange und hält an;
/// * [`Steuerung::nehmen`] (Arbeiter) nimmt den nächsten Ordner und setzt erst dann das Kennzeichen
///   für den neuen Lauf zurück.
#[derive(Default)]
pub struct Steuerung {
    zustand: Mutex<(BTreeSet<PathBuf>, bool)>,
    pub abbrechen: AtomicBool,
}

impl Steuerung {
    pub fn anstossen(&self, dir: PathBuf) {
        if let Ok(mut z) = self.zustand.lock() {
            z.1 = false;
            z.0.insert(dir);
        }
    }

    /// Wie [`Self::anstossen`], aber nicht nach einem Abbruch; `true`, wenn eingereiht.
    pub fn wiederanlauf(&self, dir: PathBuf) -> bool {
        match self.zustand.lock() {
            Ok(mut z) if !z.1 => {
                z.0.insert(dir);
                true
            }
            _ => false,
        }
    }

    pub fn abbrechen(&self) {
        if let Ok(mut z) = self.zustand.lock() {
            self.abbrechen.store(true, Ordering::Relaxed);
            z.0.clear();
            z.1 = true;
        }
    }

    pub fn angehalten(&self) -> bool {
        self.zustand.lock().map(|z| z.1).unwrap_or(false)
    }

    /// Nächster Ordner; setzt das Abbruchkennzeichen für diesen neuen Lauf zurück.
    pub fn nehmen(&self) -> Option<PathBuf> {
        let mut z = self.zustand.lock().ok()?;
        let d = z.0.iter().next().cloned()?;
        z.0.remove(&d);
        self.abbrechen.store(false, Ordering::Relaxed);
        Some(d)
    }
}

struct Schlange {
    steuerung: Steuerung,
    wecker: Mutex<()>,
    signal: Condvar,
    melden: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn schlange() -> &'static Schlange {
    static S: OnceLock<Schlange> = OnceLock::new();
    S.get_or_init(|| {
        let s = Schlange {
            steuerung: Steuerung::default(),
            wecker: Mutex::new(()),
            signal: Condvar::new(),
            melden: Mutex::new(None),
        };
        let gestartet = std::thread::Builder::new()
            .name("ablage-text".into())
            .spawn(arbeiter);
        if gestartet.is_err() {
            log::warn!("ablage: Textarbeiter nicht gestartet");
        }
        s
    })
}

/// Rückruf nach jedem Ergebnis (die Befehle senden damit ein Ereignis an die Oberfläche).
pub fn melden_setzen(f: Box<dyn Fn() + Send>) {
    if let Ok(mut m) = schlange().melden.lock() {
        *m = Some(f);
    }
}

fn wecken(s: &Schlange) {
    let _g = s.wecker.lock();
    s.signal.notify_one();
}

/// Ausdrücklicher Anstoß (Import, Änderung am Bestand): weckt den Arbeiter für die Ablage unter `dir`
/// und hebt ein Anhalten auf. Ein laufender, abgebrochener Lauf bleibt abgebrochen.
pub fn anstossen(dir: PathBuf) {
    let s = schlange();
    s.steuerung.anstossen(dir);
    wecken(s);
}

/// Wiederanlauf beim Laden der Seite: nur, wenn nicht nach einem Abbruch angehalten.
pub fn wiederanlauf(dir: PathBuf) {
    let s = schlange();
    if s.steuerung.wiederanlauf(dir) {
        wecken(s);
    }
}

/// Bricht den laufenden Extraktor und die restliche Warteschlange ab und hält an (bis zum nächsten
/// ausdrücklichen Anstoß).
pub fn abbrechen() {
    schlange().steuerung.abbrechen();
}

/// Ist die Warteschlange nach einem Abbruch angehalten?
pub fn angehalten() -> bool {
    schlange().steuerung.angehalten()
}

fn arbeiter() {
    let s = schlange();
    loop {
        let dir = {
            let Ok(mut g) = s.wecker.lock() else { return };
            loop {
                if let Some(d) = s.steuerung.nehmen() {
                    break d;
                }
                g = match s.signal.wait_timeout(g, std::time::Duration::from_secs(5)) {
                    Ok((g, _)) => g,
                    Err(_) => return,
                };
            }
        };
        let prog = match Programm::finden() {
            Ok(p) => p,
            Err(_) => {
                log::warn!("ablage: Extraktor nicht gefunden");
                continue;
            }
        };
        let melden = || {
            if let Ok(m) = s.melden.lock() {
                if let Some(f) = m.as_ref() {
                    f();
                }
            }
        };
        let a = Ablage::neu(dir);
        match lauf(
            &a,
            &prog,
            textlauf::STANDARD,
            &s.steuerung.abbrechen,
            &melden,
        ) {
            Ok(l) => log::info!(
                "ablage: Textfassungen ok={} kein_text={} fehler={} abgebrochen={}",
                l.ok,
                l.kein_text,
                l.fehler,
                l.abgebrochen
            ),
            Err(e) => log::warn!("ablage: Textlauf beendet mit {}", e.code),
        }
        // Kein Abschlussereignis: `lauf` meldet jedes Ergebnis selbst. Ein Ereignis ohne Änderung würde
        // über das Neuladen der Seite (`ablage_stand`) einen neuen leeren Lauf anstoßen.
    }
}
