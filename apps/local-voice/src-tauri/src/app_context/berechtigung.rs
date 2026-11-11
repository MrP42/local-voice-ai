//! Berechtigungsversion des Inhaltszugriffs im Sprachgespräch (lva#509, Befund 1/2).
//!
//! Der Inhaltszugriff hängt an vier Einstellungen: Hauptschalter `voice_global_context_enabled`,
//! Bereiche `voice_call_knowledge`, Unterschalter `voice_call_read_transcripts` und die
//! Notizbuch-Abschnitte `voice_call_notebook_sections`. Zwei Dinge müssen gelten:
//!
//! 1. **Freigeben ist atomar.** [`setzen_mit`] vergleicht die erwartete Version, setzt Hauptschalter,
//!    Bereiche und Transkripte in EINEM Schreibvorgang und erhöht die Version, alles unter einer
//!    Sperre. Stimmt die Version nicht (inzwischen hat jemand anders etwas geändert), wird nichts
//!    geschrieben; es gibt weder halbe Stände noch ein Zurückrollen, das einen Widerruf überschreibt.
//! 2. **Widerruf gewinnt.** Jede Änderung eines dieser Werte erhöht die Version, gleich über welchen
//!    Weg sie geschrieben wurde (die Prüfung steckt im Schreibpfad der Einstellungen). Ein laufender
//!    Abruf merkt sich die Version beim Start und gibt keinen Auszug aus, wenn sie sich bis zur Ausgabe
//!    geändert hat; ebenso verlangt das Tor vor dem Modellstart die unveränderte Version.
//!
//! **Reihenfolge und Sperren (lva#511).** Alles, was Version und Einstellungen gemeinsam betrifft,
//! läuft unter DERSELBEN Sperre [`Zaehler::sperren`]; sie ist die äußerste Sperre der Einstellungen.
//! Darunter liegt nur noch der Store der Einstellungen (`store.get`/`store.set`, intern gesperrt, ruft
//! nie zurück); unter ihr wird kein anderer Mutex genommen und kein Befehl gerufen. Die Reihenfolge der
//! Sperren im ganzen Pfad ist deshalb: `RETRIEVING` (nur `try_lock`) -> Berechtigungssperre -> Store.
//! Kein Pfad hält die Berechtigungssperre und fragt dann `RETRIEVING` oder wieder die Berechtigungssperre
//! (der Mutex ist nicht wiedereintrittsfest) – Folgearbeiten wie `on_settings_changed` laufen erst nach
//! dem Freigeben.
//!
//! - Schreiber ([`Zaehler::schreiben_mit`]): ERST speichern, DANN die Version veröffentlichen. Wer die
//!   Version ohne Sperre liest (die Endprüfungen `version() != v_start`), sieht also nie eine neue
//!   Version mit noch alten Einstellungen; eine alte Version mit neuen Einstellungen ist harmlos, denn
//!   jedes Ergebnis ist an die Version gebunden und fällt bei der Endprüfung durch.
//! - Leser, die Version UND Einstellungen brauchen ([`Zaehler::lesen_mit`]): ein Schnappschuss unter
//!   der Sperre; er zeigt nie einen halb geschriebenen Stand, also auch nie erweiterte Bereiche zu
//!   einer alten Version.
//!
//! Die Version ist ein `u32` (die Bindings kennen kein `u64`) und beginnt bei 1, damit ein fehlendes
//! Feld (`0`) nie zufällig passt. Das Modul ist rein: keine App, keine Einstellungen lesen.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::settings::AppSettings;

/// Zähler samt Sperre. Der Prozess hat genau einen ([`global`]); Tests nehmen eigene Instanzen.
#[derive(Debug)]
pub struct Zaehler {
    version: AtomicU32,
    sperre: Mutex<()>,
}

impl Zaehler {
    pub const fn neu() -> Self {
        Self {
            version: AtomicU32::new(1),
            sperre: Mutex::new(()),
        }
    }

    pub fn version(&self) -> u32 {
        self.version.load(Ordering::SeqCst)
    }

    /// Die Schreibsperre (vergiftungsfest: eine abgestürzte Schreiberin sperrt nicht für immer).
    pub fn sperren(&self) -> MutexGuard<'_, ()> {
        self.sperre.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn erhoehen(&self) -> u32 {
        self.version.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }

    /// Schreibt unter der Sperre des Aufrufers: ERST `speichere`, DANN steigt die Version (wenn sich
    /// die vier Werte unterscheiden). Liefert `true`, wenn sie stieg. Die Reihenfolge ist Absicht:
    /// ein Leser ohne Sperre, der die neue Version sieht, findet auch die neuen Einstellungen.
    pub fn schreiben_mit(&self, alt: &Stand, neu: &Stand, speichere: impl FnOnce()) -> bool {
        speichere();
        self.beim_schreiben(alt, neu)
    }

    /// Version und Einstellungen als EIN Schnappschuss: `lies` läuft unter der Sperre, die auch jeder
    /// Schreiber hält, und `lies` darf sie nicht noch einmal nehmen (kein Wiedereintritt).
    pub fn lesen_mit<R>(&self, lies: impl FnOnce() -> R) -> (u32, R) {
        let _sperre = self.sperren();
        (self.version(), lies())
    }

    /// Erhöht die Version, wenn sich die vier Werte unterscheiden. Liefert `true`, wenn sie stieg.
    /// Nur für Aufrufer, die schon gespeichert haben (sonst [`Self::schreiben_mit`]).
    pub fn beim_schreiben(&self, alt: &Stand, neu: &Stand) -> bool {
        if alt == neu {
            return false;
        }
        self.erhoehen();
        true
    }
}

static GLOBAL: Zaehler = Zaehler::neu();

/// Der prozessweite Zähler.
pub fn global() -> &'static Zaehler {
    &GLOBAL
}

/// Aktuelle Berechtigungsversion des Prozesses.
pub fn version() -> u32 {
    GLOBAL.version()
}

/// Die vier Werte, die den Inhaltszugriff bestimmen (zum Vergleichen alt gegen neu).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stand {
    pub enabled: bool,
    pub areas: Vec<String>,
    pub read_transcripts: bool,
    pub notebook_sections: Vec<String>,
}

impl Stand {
    pub fn aus_einstellungen(s: &AppSettings) -> Self {
        Self {
            enabled: s.voice_global_context_enabled,
            areas: s.voice_call_knowledge.clone(),
            read_transcripts: s.voice_call_read_transcripts,
            notebook_sections: s.voice_call_notebook_sections.clone(),
        }
    }

    /// Aus dem gespeicherten JSON der Einstellungen; fehlende Felder gelten als Standard (aus/leer),
    /// so wie sie beim Laden einer älteren Datei ergänzt würden.
    pub fn aus_json(v: Option<&Value>) -> Self {
        let Some(v) = v else {
            return Self::default();
        };
        let liste = |key: &str| -> Vec<String> {
            v.get(key)
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        let schalter = |key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
        Self {
            enabled: schalter("voice_global_context_enabled"),
            areas: liste("voice_call_knowledge"),
            read_transcripts: schalter("voice_call_read_transcripts"),
            notebook_sections: liste("voice_call_notebook_sections"),
        }
    }
}

/// Was ein GENERISCHER Schreibvorgang für die vier Werte bewirken darf: nur einschränken, nie
/// erweitern. Schalter: gespeichert UND eingehend. Listen: Schnittmenge, in der Reihenfolge des
/// Gespeicherten. Ein veralteter Lesen-Ändern-Schreiben-Zyklus kann so eine widerrufene Freigabe nicht
/// zurückbringen; umgekehrt nimmt er eine frische Freigabe wieder weg, wenn sein alter Stand sie nicht
/// kannte (die sichere Richtung). Erweitern geht nur über die gesperrten Wege.
pub fn nur_einschraenken(gespeichert: &Stand, eingehend: &Stand) -> Stand {
    let schnitt = |g: &[String], e: &[String]| -> Vec<String> {
        g.iter().filter(|x| e.contains(x)).cloned().collect()
    };
    Stand {
        enabled: gespeichert.enabled && eingehend.enabled,
        areas: schnitt(&gespeichert.areas, &eingehend.areas),
        read_transcripts: gespeichert.read_transcripts && eingehend.read_transcripts,
        notebook_sections: schnitt(&gespeichert.notebook_sections, &eingehend.notebook_sections),
    }
}

/// Wendet [`nur_einschraenken`] auf die Einstellungen an. Ist noch nichts gespeichert (`None`, erster
/// Ladevorgang), bleibt alles, wie es ist.
pub fn begrenze(gespeichert: Option<&Value>, settings: &mut AppSettings) {
    let Some(g) = gespeichert else {
        return;
    };
    let wirksam = nur_einschraenken(
        &Stand::aus_json(Some(g)),
        &Stand::aus_einstellungen(settings),
    );
    settings.voice_global_context_enabled = wirksam.enabled;
    settings.voice_call_knowledge = wirksam.areas;
    settings.voice_call_read_transcripts = wirksam.read_transcripts;
    settings.voice_call_notebook_sections = wirksam.notebook_sections;
}

/// Der gelesene Stand für die Oberfläche (`voice_call_inhaltszugriff_lesen`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Inhaltszugriff {
    pub version: u32,
    pub enabled: bool,
    pub areas: Vec<String>,
    pub read_transcripts: bool,
}

impl Inhaltszugriff {
    pub fn aus_einstellungen(version: u32, s: &AppSettings) -> Self {
        Self {
            version,
            enabled: s.voice_global_context_enabled,
            areas: s.voice_call_knowledge.clone(),
            read_transcripts: s.voice_call_read_transcripts,
        }
    }
}

/// Antwort von `voice_call_inhaltszugriff_setzen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Setzen {
    /// Geschrieben; `version` ist die neue Version.
    Ok { version: u32 },
    /// Nichts geschrieben: die erwartete Version war nicht mehr die aktuelle.
    Konflikt { version: u32 },
}

/// Setzt Hauptschalter, Bereiche und Transkripte atomar, wenn `erwartet` noch die aktuelle Version
/// ist. `lies` liefert die aktuellen Einstellungen, `schreibe` schreibt sie (und erhöht über
/// [`Zaehler::beim_schreiben`] die Version) – beides läuft unter der Sperre, die auch jeder andere
/// Schreiber der vier Werte hält.
pub fn setzen_mit(
    zaehler: &Zaehler,
    erwartet: u32,
    enabled: bool,
    areas: &[String],
    read_transcripts: bool,
    lies: impl FnOnce() -> AppSettings,
    schreibe: impl FnOnce(AppSettings),
) -> Setzen {
    let _sperre = zaehler.sperren();
    if zaehler.version() != erwartet {
        return Setzen::Konflikt {
            version: zaehler.version(),
        };
    }
    let mut settings = lies();
    settings.voice_global_context_enabled = enabled;
    settings.voice_call_knowledge = crate::agent_bridge::voice::areas::normalize_keys(areas);
    settings.voice_call_read_transcripts = read_transcripts;
    schreibe(settings);
    Setzen::Ok {
        version: zaehler.version(),
    }
}

/// Der Lese-Schreib-Zyklus der einzelnen Updater (`change_voice_*_setting`) unter derselben Sperre,
/// damit kein Updater zwischen Vergleich und Schreiben von [`setzen_mit`] dazwischenfährt.
pub fn aendern_mit<R>(zaehler: &Zaehler, f: impl FnOnce() -> R) -> R {
    let _sperre = zaehler.sperren();
    f()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn gespeichert() -> RefCell<AppSettings> {
        RefCell::new(crate::settings::get_default_settings())
    }

    /// Schreibweg wie in `settings::write_settings_unlocked`: vergleicht alt gegen neu, erhöht, speichert.
    fn schreiber<'a>(
        z: &'a Zaehler,
        store: &'a RefCell<AppSettings>,
    ) -> impl FnOnce(AppSettings) + 'a {
        move |neu| {
            let alt = Stand::aus_einstellungen(&store.borrow());
            z.beim_schreiben(&alt, &Stand::aus_einstellungen(&neu));
            *store.borrow_mut() = neu;
        }
    }

    fn bereiche() -> Vec<String> {
        vec!["meetings".into(), "books".into()]
    }

    #[test]
    fn setzen_mit_passender_version_schreibt_alles_in_einem_zug_und_erhoeht() {
        let z = Zaehler::neu();
        let store = gespeichert();
        let v0 = z.version();
        let r = setzen_mit(
            &z,
            v0,
            true,
            &bereiche(),
            true,
            || store.borrow().clone(),
            schreiber(&z, &store),
        );
        assert_eq!(r, Setzen::Ok { version: v0 + 1 });
        let s = store.borrow();
        assert!(s.voice_global_context_enabled && s.voice_call_read_transcripts);
        assert_eq!(s.voice_call_knowledge, bereiche());
    }

    #[test]
    fn veraltete_version_schreibt_nichts_und_meldet_konflikt() {
        let z = Zaehler::neu();
        let store = gespeichert();
        let v0 = z.version();
        // Zwischenzeitlich widerruft jemand: die Version steigt.
        let alt = Stand::aus_einstellungen(&store.borrow());
        let neu = Stand {
            enabled: true,
            ..alt.clone()
        };
        z.beim_schreiben(&alt, &neu);
        let vorher = store.borrow().clone();
        let r = setzen_mit(
            &z,
            v0,
            true,
            &bereiche(),
            true,
            || store.borrow().clone(),
            schreiber(&z, &store),
        );
        assert_eq!(r, Setzen::Konflikt { version: v0 + 1 });
        let s = store.borrow();
        assert_eq!(
            s.voice_global_context_enabled,
            vorher.voice_global_context_enabled
        );
        assert_eq!(s.voice_call_knowledge, vorher.voice_call_knowledge);
        assert_eq!(
            s.voice_call_read_transcripts,
            vorher.voice_call_read_transcripts
        );
    }

    #[test]
    fn zweiter_aufruf_mit_alter_version_scheitert() {
        let z = Zaehler::neu();
        let store = gespeichert();
        let v0 = z.version();
        let erster = setzen_mit(
            &z,
            v0,
            true,
            &bereiche(),
            true,
            || store.borrow().clone(),
            schreiber(&z, &store),
        );
        assert!(matches!(erster, Setzen::Ok { .. }));
        let zweiter = setzen_mit(
            &z,
            v0,
            false,
            &[],
            false,
            || store.borrow().clone(),
            schreiber(&z, &store),
        );
        assert!(matches!(zweiter, Setzen::Konflikt { .. }));
        assert!(
            store.borrow().voice_global_context_enabled,
            "der Konflikt hat nichts überschrieben"
        );
    }

    #[test]
    fn jede_aenderung_der_vier_werte_erhoeht_die_version() {
        let basis = Stand::default();
        let faelle = [
            Stand {
                enabled: true,
                ..basis.clone()
            },
            Stand {
                areas: vec!["books".into()],
                ..basis.clone()
            },
            Stand {
                read_transcripts: true,
                ..basis.clone()
            },
            Stand {
                notebook_sections: vec!["abschnitt:1".into()],
                ..basis.clone()
            },
        ];
        for neu in faelle {
            let z = Zaehler::neu();
            let v0 = z.version();
            assert!(z.beim_schreiben(&basis, &neu));
            assert_eq!(z.version(), v0 + 1, "{neu:?}");
        }
    }

    #[test]
    fn unveraenderte_werte_erhoehen_nicht() {
        let z = Zaehler::neu();
        let s = Stand {
            enabled: true,
            ..Default::default()
        };
        assert!(!z.beim_schreiben(&s, &s.clone()));
        assert_eq!(z.version(), 1);
    }

    #[test]
    fn stand_aus_json_ergaenzt_fehlende_felder_als_aus() {
        let leer = Stand::aus_json(Some(&serde_json::json!({})));
        assert_eq!(leer, Stand::default());
        assert_eq!(Stand::aus_json(None), Stand::default());
        let v = serde_json::json!({
            "voice_global_context_enabled": true,
            "voice_call_knowledge": ["books", "meetings"],
            "voice_call_read_transcripts": true,
            "voice_call_notebook_sections": ["abschnitt:7"],
        });
        let s = Stand::aus_json(Some(&v));
        assert!(s.enabled && s.read_transcripts);
        assert_eq!(s.areas, vec!["books".to_string(), "meetings".to_string()]);
        assert_eq!(s.notebook_sections, vec!["abschnitt:7".to_string()]);
    }

    #[test]
    fn json_und_einstellungen_ergeben_denselben_stand() {
        let mut s = crate::settings::get_default_settings();
        s.voice_global_context_enabled = true;
        s.voice_call_knowledge = vec!["books".into()];
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(Stand::aus_json(Some(&json)), Stand::aus_einstellungen(&s));
    }

    #[test]
    fn die_sperre_serialisiert_parallele_freigaben() {
        use std::sync::Arc;
        let z = Arc::new(Zaehler::neu());
        let store = Arc::new(Mutex::new(crate::settings::get_default_settings()));
        let v0 = z.version();
        let ok = Arc::new(AtomicU32::new(0));
        let mut handles = Vec::new();
        for i in 0..8 {
            let (z, store, ok) = (z.clone(), store.clone(), ok.clone());
            handles.push(std::thread::spawn(move || {
                let r = setzen_mit(
                    &z,
                    v0,
                    i % 2 == 0,
                    &bereiche(),
                    true,
                    || store.lock().unwrap().clone(),
                    |neu| {
                        let alt = Stand::aus_einstellungen(&store.lock().unwrap());
                        z.beim_schreiben(&alt, &Stand::aus_einstellungen(&neu));
                        *store.lock().unwrap() = neu;
                    },
                );
                if matches!(r, Setzen::Ok { .. }) {
                    ok.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            ok.load(Ordering::SeqCst),
            1,
            "genau eine Freigabe mit derselben Version gewinnt"
        );
    }
    #[test]
    fn generischer_schreibvorgang_schraenkt_nur_ein() {
        let g = Stand {
            enabled: true,
            areas: vec!["meetings".into(), "books".into()],
            read_transcripts: false,
            notebook_sections: vec!["abschnitt:1".into()],
        };
        let e = Stand {
            enabled: true,
            areas: vec!["books".into(), "dictations".into()],
            read_transcripts: true,
            notebook_sections: vec!["abschnitt:2".into(), "abschnitt:1".into()],
        };
        let w = nur_einschraenken(&g, &e);
        assert!(w.enabled);
        assert_eq!(
            w.areas,
            vec!["books".to_string()],
            "Schnittmenge, nichts Neues"
        );
        assert!(!w.read_transcripts, "Transkripte werden nicht erweitert");
        assert_eq!(w.notebook_sections, vec!["abschnitt:1".to_string()]);
    }

    #[test]
    fn listen_behalten_die_reihenfolge_des_gespeicherten() {
        let g = Stand {
            areas: vec!["meetings".into(), "books".into()],
            ..Default::default()
        };
        let e = Stand {
            areas: vec!["books".into(), "meetings".into()],
            ..Default::default()
        };
        assert_eq!(nur_einschraenken(&g, &e).areas, g.areas);
    }

    #[test]
    fn veralteter_zyklus_nach_widerruf_bringt_die_freigabe_nicht_zurueck() {
        let z = Zaehler::neu();
        // Der Zyklus las, als alles frei war ...
        let mut alt_gelesen = crate::settings::get_default_settings();
        alt_gelesen.voice_global_context_enabled = true;
        alt_gelesen.voice_call_knowledge = vec!["meetings".into()];
        alt_gelesen.voice_call_read_transcripts = true;
        // ... inzwischen ist widerrufen worden (gespeichert: alles aus).
        let gespeichert = serde_json::to_value(crate::settings::get_default_settings()).unwrap();
        let v0 = z.version();
        let mut s = alt_gelesen.clone();
        begrenze(Some(&gespeichert), &mut s);
        assert!(!s.voice_global_context_enabled);
        assert!(s.voice_call_knowledge.is_empty());
        assert!(!s.voice_call_read_transcripts);
        let neu = Stand::aus_einstellungen(&s);
        assert!(!z.beim_schreiben(&Stand::aus_json(Some(&gespeichert)), &neu));
        assert_eq!(
            z.version(),
            v0,
            "wirkungsloses Schreiben erhöht die Version nicht"
        );
    }

    #[test]
    fn veralteter_zyklus_nach_freigabe_nimmt_sie_wieder_weg_sichere_richtung() {
        let mut frisch = crate::settings::get_default_settings();
        frisch.voice_global_context_enabled = true;
        let gespeichert = serde_json::to_value(&frisch).unwrap();
        let mut alt_gelesen = crate::settings::get_default_settings();
        alt_gelesen.voice_global_context_enabled = false;
        begrenze(Some(&gespeichert), &mut alt_gelesen);
        assert!(!alt_gelesen.voice_global_context_enabled);
    }

    #[test]
    fn beim_ersten_laden_ohne_gespeichertes_bleibt_alles_wie_es_ist() {
        let mut s = crate::settings::get_default_settings();
        s.voice_global_context_enabled = true;
        s.voice_call_knowledge = vec!["books".into()];
        begrenze(None, &mut s);
        assert!(s.voice_global_context_enabled);
        assert_eq!(s.voice_call_knowledge, vec!["books".to_string()]);
    }

    #[test]
    fn schreiben_ohne_aenderung_der_vier_werte_erhoeht_nichts() {
        let z = Zaehler::neu();
        let mut s = crate::settings::get_default_settings();
        s.voice_global_context_enabled = true;
        let gespeichert = serde_json::to_value(&s).unwrap();
        let mut e = s.clone();
        begrenze(Some(&gespeichert), &mut e);
        assert!(!z.beim_schreiben(
            &Stand::aus_json(Some(&gespeichert)),
            &Stand::aus_einstellungen(&e)
        ));
    }

    // ---- Reihenfolge Speichern -> Version veröffentlichen, Leser ohne und mit Sperre (lva#511) ----

    /// Zustand Nummer `n` des Spiels: gerade = alles frei (Bereiche, Transkripte), ungerade = nichts.
    /// Die Nummer steckt in `notebook_sections`, damit ein Leser erkennt, welcher Stand er sieht.
    fn zustand(n: u32) -> Stand {
        let frei = n % 2 == 0;
        Stand {
            enabled: frei,
            areas: if frei { bereiche() } else { Vec::new() },
            read_transcripts: frei,
            notebook_sections: vec![n.to_string()],
        }
    }

    fn nummer(s: &Stand) -> u32 {
        s.notebook_sections[0].parse().unwrap()
    }

    /// Ein Schreiber wie in `settings::schreibe_unter_sperre`: Sperre, nächsten Zustand bilden, erst
    /// speichern, dann die Version veröffentlichen.
    fn naechster_zustand(z: &Zaehler, store: &Mutex<Stand>) {
        let _sperre = z.sperren();
        let neu = zustand(z.version() + 1);
        let alt = store.lock().unwrap().clone();
        assert!(z.schreiben_mit(&alt, &neu, || *store.lock().unwrap() = neu.clone()));
    }

    #[test]
    fn die_version_steigt_erst_nach_dem_speichern() {
        let z = Zaehler::neu();
        let store = Mutex::new(zustand(1));
        let alt = store.lock().unwrap().clone();
        let neu = zustand(2);
        let beim_speichern = RefCell::new(None);
        let gestiegen = z.schreiben_mit(&alt, &neu, || {
            // Während des Speicherns ist die neue Version noch nicht zu sehen.
            *beim_speichern.borrow_mut() = Some(z.version());
            *store.lock().unwrap() = neu.clone();
        });
        assert!(gestiegen);
        assert_eq!(*beim_speichern.borrow(), Some(1));
        assert_eq!(z.version(), 2);
        assert_eq!(*store.lock().unwrap(), neu);
    }

    #[test]
    fn unveraenderte_werte_speichern_ohne_neue_version() {
        let z = Zaehler::neu();
        let s = zustand(1);
        let gespeichert = RefCell::new(false);
        assert!(!z.schreiben_mit(&s, &s.clone(), || *gespeichert.borrow_mut() = true));
        assert!(*gespeichert.borrow());
        assert_eq!(z.version(), 1);
    }

    /// BEWEIS der Reihenfolge: die Zwischenphase "gespeichert, Version noch nicht veröffentlicht" wird
    /// erzwungen (der Schreiber wartet in `speichere` auf ein "weiter"), ohne Zeitglück.
    #[test]
    fn erzwungene_zwischenphase_speichern_fertig_version_noch_alt() {
        use std::sync::{mpsc, Arc};
        use std::time::Duration;
        let z = Arc::new(Zaehler::neu());
        let store = Arc::new(Mutex::new(zustand(1)));
        let (zwischen_tx, zwischen_rx) = mpsc::channel::<()>();
        let (weiter_tx, weiter_rx) = mpsc::channel::<()>();

        let schreiber = {
            let (z, store) = (z.clone(), store.clone());
            std::thread::spawn(move || {
                let _sperre = z.sperren();
                let alt = store.lock().unwrap().clone();
                let neu = zustand(2);
                z.schreiben_mit(&alt, &neu, || {
                    assert_eq!(z.version(), 1, "vor dem Speichern ist die Version alt");
                    *store.lock().unwrap() = neu.clone();
                    assert_eq!(
                        z.version(),
                        1,
                        "nach dem Speichern, vor der Veröffentlichung, ist die Version noch alt"
                    );
                    zwischen_tx.send(()).unwrap();
                    weiter_rx.recv().unwrap();
                })
            })
        };

        // Bei vertauschter Reihenfolge fällt der Schreiber in der Closure durch: der Sender verfällt.
        zwischen_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("der Schreiber erreichte die Zwischenphase nicht");

        // Zwischenphase: ohne Sperre ist die Version noch die alte, der Stand schon der neue (sichere
        // Richtung).
        assert_eq!(z.version(), 1);
        assert_eq!(*store.lock().unwrap(), zustand(2));

        // Ein Schnappschuss-Leser wartet auf die Sperre des Schreibers: kein Ergebnis in der Zwischenphase.
        let (leser_tx, leser_rx) = mpsc::channel();
        let leser = {
            let (z, store) = (z.clone(), store.clone());
            std::thread::spawn(move || {
                leser_tx
                    .send(z.lesen_mit(|| store.lock().unwrap().clone()))
                    .unwrap();
            })
        };
        assert_eq!(
            leser_rx.recv_timeout(Duration::from_millis(200)),
            Err(mpsc::RecvTimeoutError::Timeout),
            "der Schnappschuss darf in der Zwischenphase nicht ankommen"
        );

        weiter_tx.send(()).unwrap();
        assert!(schreiber.join().unwrap(), "die Version muss gestiegen sein");
        let (version, stand) = leser_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("der Leser bekam nach dem Schreiben kein Ergebnis");
        leser.join().unwrap();
        assert_eq!((version, stand), (2, zustand(2)));
        assert_eq!(z.version(), 2);
    }

    /// RAUCHTEST unter Last, kein Beweis der Reihenfolge (das leistet
    /// `erzwungene_zwischenphase_speichern_fertig_version_noch_alt`): Leser und Schreiber laufen
    /// gleichzeitig; ob ein Leser je in die Zwischenphase fällt, hängt von der Einplanung ab.
    #[test]
    fn rauchtest_unter_last_leser_und_schreiber_parallel_ohne_unstimmigen_stand() {
        use std::sync::atomic::AtomicBool;
        use std::sync::{Arc, Barrier};
        use std::time::{Duration, Instant};
        const SCHREIBER: usize = 4;
        const LESER: usize = 4;
        /// Jeder Leser muss so viele Lesungen gemacht haben, WÄHREND Schreiber liefen.
        const MIN_LESUNGEN: u32 = 50;
        const GRENZE: Duration = Duration::from_secs(20);

        let z = Arc::new(Zaehler::neu());
        let store = Arc::new(Mutex::new(zustand(1)));
        let start = Arc::new(Barrier::new(SCHREIBER + LESER));
        // Schreiber, die noch laufen, und Leser, die ihre Mindestzahl überlappender Lesungen erreicht haben.
        let laufend = Arc::new(AtomicU32::new(SCHREIBER as u32));
        let genug = Arc::new(AtomicU32::new(0));
        let fertig = Arc::new(AtomicBool::new(false));
        let geschrieben = Arc::new(AtomicU32::new(0));

        // Schreiber: laufen in Runden weiter, bis jeder Leser genug überlappende Lesungen hat. Wird die
        // Grenze erreicht (ein Leser kommt nie zum Zug), schlägt der Test fehl.
        let schreiber: Vec<_> = (0..SCHREIBER)
            .map(|_| {
                let (z, store, start) = (z.clone(), store.clone(), start.clone());
                let (laufend, genug, geschrieben) =
                    (laufend.clone(), genug.clone(), geschrieben.clone());
                std::thread::spawn(move || {
                    start.wait();
                    let ende = Instant::now() + GRENZE;
                    while genug.load(Ordering::SeqCst) < LESER as u32 {
                        assert!(
                            Instant::now() < ende,
                            "die Leser kamen nicht zu {MIN_LESUNGEN} Lesungen während der Schreiber"
                        );
                        naechster_zustand(&z, &store);
                        geschrieben.fetch_add(1, Ordering::SeqCst);
                    }
                    laufend.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();

        // Ein Leser zählt eine Lesung nur, wenn vor UND nach ihr noch ein Schreiber lief.
        let leser = |mit_sperre: bool| {
            let (z, store, start) = (z.clone(), store.clone(), start.clone());
            let (laufend, genug, fertig) = (laufend.clone(), genug.clone(), fertig.clone());
            std::thread::spawn(move || {
                start.wait();
                let (mut ueberlappend, mut hoechste) = (0u32, 0u32);
                while !fertig.load(Ordering::SeqCst) {
                    let vorher = laufend.load(Ordering::SeqCst);
                    let version = if mit_sperre {
                        // Schnappschuss (`lesen_mit`, wie Abruf und Tor): Version und Stand passen
                        // immer zusammen, also nie erweiterte Bereiche zu einer alten und nie alte
                        // Einstellungen zu einer neuen Version.
                        let (v, s) = z.lesen_mit(|| store.lock().unwrap().clone());
                        assert_eq!(s, zustand(v), "Version {v} mit fremdem Stand");
                        v
                    } else {
                        // Ohne Sperre (die Endprüfungen): erst die Version, dann der Stand; der Stand
                        // ist nie älter als die Version.
                        let v = z.version();
                        let s = store.lock().unwrap().clone();
                        assert!(
                            nummer(&s) >= v,
                            "neue Version {v} mit altem Stand {}",
                            nummer(&s)
                        );
                        v
                    };
                    hoechste = hoechste.max(version);
                    if vorher > 0 && laufend.load(Ordering::SeqCst) > 0 {
                        ueberlappend += 1;
                        if ueberlappend == MIN_LESUNGEN {
                            genug.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                }
                (ueberlappend, hoechste)
            })
        };
        let leser: Vec<_> = (0..LESER).map(|i| leser(i % 2 == 0)).collect();

        let schreiber_ergebnis: Vec<_> = schreiber.into_iter().map(|h| h.join()).collect();
        fertig.store(true, Ordering::SeqCst);
        let lesungen: Vec<(u32, u32)> = leser.into_iter().map(|h| h.join().unwrap()).collect();
        for r in schreiber_ergebnis {
            r.unwrap();
        }

        for (i, (ueberlappend, _)) in lesungen.iter().enumerate() {
            assert!(
                *ueberlappend >= MIN_LESUNGEN,
                "Leser {i}: nur {ueberlappend} Lesungen während der Schreiber"
            );
        }
        assert!(
            lesungen.iter().any(|(_, hoechste)| *hoechste > 1),
            "keine Lesung sah eine Version über der Startversion"
        );
        let endversion = 1 + geschrieben.load(Ordering::SeqCst);
        assert_eq!(z.version(), endversion);
        assert_eq!(*store.lock().unwrap(), zustand(endversion));
    }
}
