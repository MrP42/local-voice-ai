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

    /// Beim Schreiben der Einstellungen: unterscheiden sich die vier Werte, steigt die Version.
    /// Liefert `true`, wenn sie stieg.
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
}
