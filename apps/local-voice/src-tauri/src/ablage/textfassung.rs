//! Warteschlange der Textfassungen (Spec §4.1): ein Hintergrundarbeiter, abbrechbar. Nach dem Ablegen
//! oder einer neuen Fassung steht die Datei auf `ausstehend`; der Arbeiter erzeugt die Textfassung im
//! Unterprozess, schreibt sie atomar nach `text/<dat>/<textrev>.json`, trägt sie in `texte` der Fassung
//! ein (`ok`, `kein_text` oder `fehler` mit Code) und gleicht den Index ab.
//!
//! Wiederanlauf: Es gibt keine eigene Warteschlangendatei. Offen ist, was im Stand fehlt
//! ([`Ablage::text_auftraege`]); nach einem Neustart läuft der Arbeiter einfach erneut. Ein
//! fehlgeschlagener Lauf wird als `fehler` vermerkt und nicht endlos wiederholt (eine ZIP-Bombe bleibt
//! eine ZIP-Bombe); eine neue Extraktorversion ergibt eine neue `textrev` und damit einen neuen Versuch.
//! Nie Inhalt im Log, nur Zähler und Codes.

use super::extrakt::Textfassung;
use super::speicher::{Ablage, TextAuftrag};
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
}

/// Ein Auftrag: Unterprozess, Ergebnis schreiben, eintragen.
pub fn bearbeiten(
    a: &Ablage,
    auf: &TextAuftrag,
    prog: &Programm,
    grenzen: Grenzen,
    abbrechen: &AtomicBool,
) -> Res<Ausgang> {
    let ergebnis = textlauf::ausfuehren(prog, &auf.pfad, &auf.sha256, &auf.typ, grenzen, abbrechen);
    let text = match ergebnis {
        Ok(t) if t.textrev == auf.textrev => {
            let Ok(json) = serde_json::to_vec(&t) else {
                return Ok(Ausgang::Uebersprungen);
            };
            match a.text_schreiben(&auf.dat, &t.textrev, &json) {
                Ok((blob, size)) => text_aus(&t, blob, size),
                Err(_) => return Ok(Ausgang::Uebersprungen),
            }
        }
        Ok(_) => fehler_text(auf, "extrakt.antwort"),
        Err(code) if code == "extrakt.abgebrochen" => return Ok(Ausgang::Abgebrochen),
        Err(code) => fehler_text(auf, &code),
    };
    a.text_eintragen(&auf.dat, &auf.fas, text.clone())?;
    Ok(Ausgang::Eingetragen(text))
}

/// Aufträge, deren Ergebnis in diesem Prozess nicht speicherbar war (erst der nächste Start versucht
/// sie erneut; sonst liefe der Arbeiter bei jedem Neuladen der Seite wieder los).
static UEBERSPRUNGEN: Mutex<BTreeSet<(String, String)>> = Mutex::new(BTreeSet::new());

fn uebersprungen() -> BTreeSet<(String, String)> {
    UEBERSPRUNGEN.lock().map(|u| u.clone()).unwrap_or_default()
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
                Ausgang::Eingetragen(_) => l.fehler += 1,
                Ausgang::Uebersprungen => {
                    // Nichts geändert: kein Ereignis, und in diesem Prozess nicht erneut versuchen.
                    l.fehler += 1;
                    if let Ok(mut u) = UEBERSPRUNGEN.lock() {
                        u.insert((auf.fas.clone(), auf.textrev.clone()));
                    }
                    continue;
                }
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

struct Schlange {
    offen: Mutex<BTreeSet<PathBuf>>,
    signal: Condvar,
    abbrechen: AtomicBool,
    melden: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn schlange() -> &'static Schlange {
    static S: OnceLock<Schlange> = OnceLock::new();
    S.get_or_init(|| {
        let s = Schlange {
            offen: Mutex::new(BTreeSet::new()),
            signal: Condvar::new(),
            abbrechen: AtomicBool::new(false),
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

/// Weckt den Arbeiter für die Ablage unter `dir` (nach Ablegen, neuer Fassung, Wiederherstellen, Start).
pub fn anstossen(dir: PathBuf) {
    let s = schlange();
    s.abbrechen.store(false, Ordering::Relaxed);
    if let Ok(mut o) = s.offen.lock() {
        o.insert(dir);
    }
    s.signal.notify_one();
}

/// Bricht den laufenden Extraktor und die restliche Warteschlange ab (bis zum nächsten Anstoß).
pub fn abbrechen() {
    let s = schlange();
    s.abbrechen.store(true, Ordering::Relaxed);
    if let Ok(mut o) = s.offen.lock() {
        o.clear();
    }
}

fn arbeiter() {
    let s = schlange();
    loop {
        let dir = {
            let Ok(mut o) = s.offen.lock() else { return };
            loop {
                if let Some(d) = o.iter().next().cloned() {
                    o.remove(&d);
                    break d;
                }
                o = match s.signal.wait(o) {
                    Ok(g) => g,
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
        match lauf(&a, &prog, textlauf::STANDARD, &s.abbrechen, &melden) {
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
