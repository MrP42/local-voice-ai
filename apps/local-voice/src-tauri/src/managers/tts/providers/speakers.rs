//! Sprecher eines Anbieters: aus seiner Stimmenliste die Namen, die ein Marker
//! `<Name>` im Vorlesetext treffen darf (Paket "mehrere Sprecher", 27.10.2026).
//!
//! Bei Fish sind die Sprecher die aufgenommenen Stimmen der Registry; bei einem
//! Anbieter mit [`ProviderCaps::voice_switching`](super::contract::ProviderCaps)
//! (Phonon, generischer Sprachserver, Ollama) sind es die Stimmen seiner
//! `list_voices`. Dieses Modul ist rein (kein Prozess, kein Netz, keine
//! Einstellungen) und die EINE Quelle für beide Seiten: der Parser im Backend
//! (`protocol::split_speaker_segments`, über [`ProviderSpeaker::known`]) und die
//! Oberfläche (Befehl `tts_provider_speakers`) bekommen dieselbe Liste. Der Editor
//! färbt damit nie mehr ein, als die Pipeline später schaltet.
//!
//! ## Regeln
//!
//! * Bekannt sind die SICHTBAREN Stimmen ([`visible_voice`]: Auswahl "im Vorlesen
//!   anzeigen", die gewählte Stimme bleibt dabei) ohne die Standardstimme `default`
//!   — sie ist kein Sprecher, sondern das, was Text ohne Marker liest.
//! * Name im Marker: der Teil der Beschriftung vor " – " (`Nele – Rounded, open
//!   tone` → `Nele`). Zusätzlich passt die Kennung der Stimme und die ganze
//!   Beschriftung (Alias). Abgleich wie bei Fish (`fold_speaker_name`: klein,
//!   Umlaute ausgeschrieben).
//! * Eindeutig: trägt eine spätere Stimme denselben Namen wie eine frühere, heißt
//!   sie im Marker `Name (Kennung)`; die frühere behält den schlichten Namen.
//! * Ein Name, der sich nicht als Marker schreiben lässt (`<`, `>`, `:`,
//!   Zeilenumbruch, länger als 60 Zeichen), fällt auf die Kennung zurück; geht auch
//!   die nicht, fehlt die Stimme in der Liste (sie bleibt als feste Stimme wählbar).
//!
//! ## Schlüssel statt Kennung
//!
//! Der Parser reicht an die Pipeline einen Schlüssel `provider:<anbieter>:<stimme>`
//! ([`speaker_key`]). Er trennt die Welten: ein Satz, der mit Phonon-Stimme
//! berechnet wurde, erreicht nach einem Wechsel der Engine nie den Fish-Server mit
//! einer fremden Kennung ([`voice_of_key`], [`is_provider_key`]); er liest dann in
//! der eingestellten Stimme.
//!
//! ## Fehlerfälle und ihre Absicherung
//!
//! | Fall | Absicherung |
//! |---|---|
//! | Stimm- oder Konfigurationswechsel (Dropdown) während ein Satz mit Sprecherstimme rechnet; zwei gleichzeitige Leseversuche der Liste | Die Stimme gehört zur Identität des Satzes (`SynthIdentity`: Tag des Anbieters trägt die Stimme): ein Wechsel verwirft die Berechnung und rechnet mit der neuen Stimme neu; Tests `der_wechsel_zur_festen_stimme_aendert_die_identitaet_des_satzes`, `ein_wechsel_der_anbieter_stimme_mitten_in_der_berechnung_gilt_sofort`. Die gehaltene Liste liegt hinter einem `Mutex`, der nie über `await` gehalten wird; zwei Leseversuche tun doppelte Arbeit, der letzte schreibt dieselben Daten |
//! | Abbruch mitten im Vorgang (Lesen der Liste, Satz) | Das Lesen der Liste hat eine Frist (`pipeline::VOICES_TIMEOUT`, 8 s; fällt das Future, endet auch der Kindprozess von `ptts --list-voices` über den Wächter); der Satz bricht wie bisher binnen 25 ms ab (`abbruch_mitten_in_der_synthese…`). Ein hängender Anbieter hält Aufträge nur einmal auf: nach einem Fehlschlag kommt der nächste Versuch erst nach [`ROSTER_RETRY`]; Test `ein_gescheitertes_lesen_behaelt_die_letzte_liste…` |
//! | Voller Speicher oder Datenträger | Die Liste ist klein (höchstens 1000 Stimmen, Marker höchstens 60 Zeichen); der Audio-Cache je Stimme nutzt den vorhandenen begrenzten RAM-Cache (200 MiB, FIFO) und den Platten-Cache mit `prune_disk_cache` — mehr Stimmen heißt mehr Einträge unter denselben Grenzen, keine neue Persistenz |
//! | Fehlendes Programm oder Gerät, gesperrter Anbieter | `list_voices` scheitert (`Unavailable`/`LicenseBlocked`) → leere Liste: Marker sind unbekannt, werden gestrichen und nie vorgelesen; die Synthese meldet den Grund wie bisher. Die gehaltene Liste eines anderen Anbieters oder einer anderen Konfiguration wird nie übernommen (Test `ein_gescheitertes_lesen…`) |
//! | Absturz oder Hänger des Kindprozesses | Phonon startet für die Liste nur mit Netzfreigabe einen Prozess (`--list-voices`, Frist 7 s unter den 8 s von außen, Job-Objekt-Deckel); die Synthese-Fälle deckt der Adapter (`ein_anbieterfehler_endet_im_fehlerzustand…`) |
//! | Engine-Wechsel mitten im Vorlesen (Phonon → Fish) | Schlüssel `provider:<anbieter>:<stimme>`: Fish bekommt nie eine Anbieter-Kennung, ein Anbieter nur seine eigene; Test `fremde_stimmen_erreichen_weder_den_anbieter_noch_fish` |
//! | Alte Fish-Stimme in `tts_voice`, Anbieter gewählt | Der Anbieter ignoriert sie (`effective_voice`), `fixed_voice` hängt am Anbieter (`voice_is_chosen`); Tests `die_stimme_kommt_vom_anbieter_nicht_von_fish`, `eine_feste_stimme_liest_alles…` |
//! | Gleiche oder unschreibbare Namen | `Name (Kennung)` bzw. Rückfall auf die Kennung bzw. nicht in der Liste; Tests `gleiche_namen…`, `namen_die_sich_nicht_als_marker_schreiben_lassen…` |
//! | Echtzeit-Audiopfad | Unberührt: die Schicht läuft nie im Audio-Callback; das Ergebnis eines Satzes ist wie bisher ein fertiges WAV im Player |

use std::time::{Duration, Instant};

use serde::Serialize;
use specta::Type;

use super::contract::VoiceDescriptor;
use super::policy::{visible_voice, DEFAULT_VOICE_ID};
use crate::managers::tts::engine::PROVIDER_PREFIX;
use crate::managers::tts::protocol::{fold_speaker_name, KnownSpeaker};

/// Längste Zeichenzahl innerhalb von `<…>` (wie `scan_marker_candidates`).
const MAX_MARKER_CHARS: usize = 60;

/// Wie lange die Sprecher eines Anbieters als aktuell gelten. Die Liste kommt vom
/// Anbieter (Phonon: Ordner `voices/`, Sprachserver: HTTP) und ändert sich selten;
/// nicht jeder Auftrag liest sie neu. Änderungen der Auswahl oder der Konfiguration
/// verwerfen sie sofort (`TtsManager::invalidate_provider_roster`).
pub const ROSTER_TTL: Duration = Duration::from_secs(60);
/// Nach einem gescheiterten Lesen erst nach dieser Zeit wieder versuchen: ein
/// hängender Anbieter darf nicht jeden Auftrag um seine Frist verzögern.
pub const ROSTER_RETRY: Duration = Duration::from_secs(10);

/// Die Sprecher des aktiven Anbieters, einmal gelesen und gehalten. Das Zerlegen des
/// Textes ist synchron, die Stimmenliste des Anbieters nicht: sie wird vorher gelesen
/// (`TtsManager::ensure_provider_roster`). Dieses Stück ist die reine Regel dahinter.
#[derive(Debug, Clone)]
pub struct RosterCache {
    pub provider_id: String,
    /// Stand der Konfiguration des Anbieters (sein Cache-Tag: Modell, Adresse,
    /// gewählte Stimme); ändert er sich, gilt die Liste nicht mehr.
    pub config_key: String,
    pub fetched: Instant,
    pub ttl: Duration,
    pub speakers: Vec<ProviderSpeaker>,
}

impl RosterCache {
    /// Gilt die Liste für diesen Anbieter in dieser Konfiguration noch?
    pub fn is_fresh(&self, provider_id: &str, config_key: &str, now: Instant) -> bool {
        self.provider_id == provider_id
            && self.config_key == config_key
            && now.saturating_duration_since(self.fetched) < self.ttl
    }

    /// Der neue Stand nach einem Leseversuch. `read = Some(..)`: gelungen, gilt
    /// [`ROSTER_TTL`]. `None`: gescheitert; die letzte Liste desselben Anbieters in
    /// derselben Konfiguration bleibt (besser als keine), der nächste Versuch kommt
    /// nach [`ROSTER_RETRY`]. Eine Liste eines anderen Anbieters oder einer anderen
    /// Konfiguration wird nie übernommen.
    pub fn after_read(
        previous: Option<&RosterCache>,
        provider_id: &str,
        config_key: &str,
        read: Option<Vec<ProviderSpeaker>>,
        now: Instant,
    ) -> RosterCache {
        let (speakers, ttl) = match read {
            Some(speakers) => (speakers, ROSTER_TTL),
            None => (
                previous
                    .filter(|c| c.provider_id == provider_id && c.config_key == config_key)
                    .map(|c| c.speakers.clone())
                    .unwrap_or_default(),
                ROSTER_RETRY,
            ),
        };
        RosterCache {
            provider_id: provider_id.to_string(),
            config_key: config_key.to_string(),
            fetched: now,
            ttl,
            speakers,
        }
    }
}

/// Ein Sprecher eines Anbieters, so wie ihn die Oberfläche braucht.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
pub struct ProviderSpeaker {
    /// Kennung der Stimme beim Anbieter (`SynthesisOptions::voice`).
    pub id: String,
    /// Name im Marker (`<Name>`) und in der Anzeige.
    pub name: String,
    /// Weitere Schreibweisen, die denselben Sprecher treffen (die ganze Beschriftung).
    pub aliases: Vec<String>,
    /// `female`/`male`/… wie vom Anbieter gemeldet (`None` = unbekannt).
    pub gender: Option<String>,
    /// Sprachkürzel der Stimme (`und` = beliebig).
    pub language: String,
}

impl ProviderSpeaker {
    /// Für den Parser: Schlüssel als Kennung, dazu alle Namen, die treffen.
    pub fn known(&self, provider_id: &str) -> KnownSpeaker {
        let mut names = vec![self.id.clone(), self.name.clone()];
        names.extend(self.aliases.iter().cloned());
        KnownSpeaker {
            id: speaker_key(provider_id, &self.id),
            names,
        }
    }
}

/// Schlüssel eines Anbieter-Sprechers in der Pipeline: `provider:<anbieter>:<stimme>`.
pub fn speaker_key(provider_id: &str, voice_id: &str) -> String {
    format!("{PROVIDER_PREFIX}{provider_id}:{voice_id}")
}

/// Ist `voice` ein Schlüssel eines Anbieter-Sprechers (und keine Fish-Kennung)?
pub fn is_provider_key(voice: &str) -> bool {
    voice.starts_with(PROVIDER_PREFIX)
}

/// Die Stimme beim Anbieter `provider_id` aus einem Schlüssel; `None`, wenn der
/// Schlüssel zu einem anderen Anbieter gehört (oder keiner ist).
pub fn voice_of_key<'a>(provider_id: &str, key: &'a str) -> Option<&'a str> {
    let rest = key.strip_prefix(PROVIDER_PREFIX)?;
    let voice = rest.strip_prefix(provider_id)?.strip_prefix(':')?;
    (!voice.trim().is_empty()).then_some(voice)
}

/// Ist beim Anbieter eine BESTIMMTE Stimme gewählt (`tts_providers.voices`)? Dann liest
/// sie den ganzen Text, und die Sprecher im Skript werden übergangen. Leer und
/// `default` sind keine Wahl: dann gilt "Skript mit Sprechern" (Marker schalten, Text
/// ohne Marker liest die Standardstimme des Anbieters) — wie bei Fish ohne `tts_voice`.
pub fn voice_is_chosen(chosen: Option<&String>) -> bool {
    chosen.is_some_and(|v| {
        let v = v.trim();
        !v.is_empty() && v != DEFAULT_VOICE_ID
    })
}

/// Der Name des Sprechers zu einem Schlüssel (für die Herkunftsnotiz einer Aufnahme);
/// `None`, wenn der Schlüssel zu einem anderen Anbieter gehört oder die Stimme nicht
/// (mehr) in der Liste steht.
pub fn name_of_key<'a>(
    provider_id: &str,
    speakers: &'a [ProviderSpeaker],
    key: &str,
) -> Option<&'a str> {
    let voice = voice_of_key(provider_id, key)?;
    speakers
        .iter()
        .find(|s| s.id == voice)
        .map(|s| s.name.as_str())
}

/// Der Teil einer Beschriftung vor " – " (`Nele – Rounded, open tone` → `Nele`).
pub fn short_name(label: &str) -> &str {
    let name = label.split(" – ").next().unwrap_or(label).trim();
    if name.is_empty() {
        label.trim()
    } else {
        name
    }
}

/// Lässt sich `name` als `<name>` in einen Text schreiben, ohne dass der Parser
/// ihn anders liest (Stil am `:`, Klammern, Zeilenumbruch, Länge)?
fn is_marker_safe(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= MAX_MARKER_CHARS
        && !name.contains(['<', '>', ':', '\n', '\r'])
}

/// Die Sprecher aus einer Stimmenliste (Reihenfolge des Anbieters bleibt).
/// `visible`: gespeicherte Auswahl "im Vorlesen anzeigen" des Anbieters (leer oder
/// `None` = alle); `selected`: die gewählte Stimme, die immer sichtbar bleibt.
pub fn build_roster(
    voices: &[VoiceDescriptor],
    visible: Option<&Vec<String>>,
    selected: Option<&str>,
) -> Vec<ProviderSpeaker> {
    let mut roster: Vec<ProviderSpeaker> = Vec::new();
    // Vergleichsformen aller schon vergebenen Namen und Kennungen.
    let mut claimed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for voice in voices {
        let id = voice.id.trim();
        if id.is_empty()
            || id == DEFAULT_VOICE_ID
            || !visible_voice(visible, selected, &voice.id)
            || roster.iter().any(|s| s.id == voice.id)
        {
            continue;
        }
        let short = short_name(&voice.label);
        let base = if is_marker_safe(short) {
            short.to_string()
        } else if is_marker_safe(id) {
            id.to_string()
        } else {
            continue;
        };
        let name = if claimed.contains(&fold_speaker_name(&base)) {
            let with_id = format!("{base} ({id})");
            if !is_marker_safe(&with_id) || claimed.contains(&fold_speaker_name(&with_id)) {
                continue;
            }
            with_id
        } else {
            base
        };
        claimed.insert(fold_speaker_name(&name));
        // Die ganze Beschriftung als weitere Schreibweise, sofern sie frei und
        // schreibbar ist und nichts Neues trifft, was schon einem anderen gehört.
        let mut aliases = Vec::new();
        let label = voice.label.trim();
        if label != name && is_marker_safe(label) && !claimed.contains(&fold_speaker_name(label)) {
            claimed.insert(fold_speaker_name(label));
            aliases.push(label.to_string());
        }
        roster.push(ProviderSpeaker {
            id: voice.id.clone(),
            name,
            aliases,
            gender: voice.gender.clone(),
            language: voice.language.clone(),
        });
    }
    roster
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::tts::protocol::split_speaker_segments;

    fn voice(id: &str, label: &str, gender: Option<&str>) -> VoiceDescriptor {
        VoiceDescriptor {
            id: id.to_string(),
            label: label.to_string(),
            language: "de".to_string(),
            gender: gender.map(String::from),
            license: None,
        }
    }

    /// Eine Liste wie die von Phonon mit `voices/SOURCE.json`: Standardstimme,
    /// zwei benannte Stimmen mit Kurzbeschreibung.
    fn phonon_like() -> Vec<VoiceDescriptor> {
        vec![
            voice("default", "Standardstimme des Modells", None),
            voice(
                "2A2fFf8SkDEJ8nzx",
                "Nele – Rounded, open tone",
                Some("female"),
            ),
            voice("AAAA", "Anton – Warm baritone", Some("male")),
        ]
    }

    #[test]
    fn die_standardstimme_ist_kein_sprecher_und_die_namen_kommen_vor_dem_gedankenstrich() {
        let roster = build_roster(&phonon_like(), None, None);
        let names: Vec<&str> = roster.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Nele", "Anton"]);
        assert_eq!(roster[0].id, "2A2fFf8SkDEJ8nzx");
        assert_eq!(roster[0].gender.as_deref(), Some("female"));
        assert_eq!(roster[0].aliases, ["Nele – Rounded, open tone"]);
    }

    #[test]
    fn nur_sichtbare_stimmen_sind_sprecher_die_gewaehlte_bleibt() {
        let visible = vec!["AAAA".to_string()];
        let roster = build_roster(&phonon_like(), Some(&visible), None);
        assert_eq!(
            roster.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            ["Anton"]
        );
        let with_selected = build_roster(&phonon_like(), Some(&visible), Some("2A2fFf8SkDEJ8nzx"));
        assert_eq!(
            with_selected.len(),
            2,
            "die gewählte Stimme bleibt sichtbar"
        );
        // Eine leere Auswahl heißt: alle.
        assert_eq!(
            build_roster(&phonon_like(), Some(&Vec::new()), None).len(),
            2
        );
    }

    #[test]
    fn gleiche_namen_werden_mit_der_kennung_unterschieden() {
        let voices = vec![
            voice("a1", "Nele – hell", None),
            voice("b2", "Nele – dunkel", None),
        ];
        let roster = build_roster(&voices, None, None);
        assert_eq!(roster[0].name, "Nele");
        assert_eq!(roster[1].name, "Nele (b2)");
        // Beide sind ansprechbar, jeder trifft seine Stimme.
        let known: Vec<KnownSpeaker> = roster.iter().map(|s| s.known("phonon")).collect();
        let segs = split_speaker_segments("<Nele> Eins. <Nele (b2)> Zwei.", &known);
        assert_eq!(segs[0].voice.as_deref(), Some("provider:phonon:a1"));
        assert_eq!(segs[1].voice.as_deref(), Some("provider:phonon:b2"));
    }

    #[test]
    fn namen_die_sich_nicht_als_marker_schreiben_lassen_fallen_auf_die_kennung_zurueck() {
        let voices = vec![
            voice("ok-id", "Wer: ich <bin>", None),
            voice("a:b", "Zeile:Zwei", None),
        ];
        let roster = build_roster(&voices, None, None);
        assert_eq!(roster.len(), 1, "die Kennung a:b taugt nicht als Marker");
        assert_eq!(roster[0].name, "ok-id");
    }

    #[test]
    fn marker_treffen_name_kennung_und_beschriftung_wie_bei_fish() {
        let roster = build_roster(&phonon_like(), None, None);
        let known: Vec<KnownSpeaker> = roster.iter().map(|s| s.known("phonon")).collect();
        let text = "<NELE> Guten Morgen.\n<2a2fff8skdejz8nx> unbekannt\n<Anton:leise> Hallo.\n\
                    <Anton – Warm baritone> Nochmal.\n<Bob> Wer bin ich?";
        let segs = split_speaker_segments(text, &known);
        let voices: Vec<Option<&str>> = segs.iter().map(|s| s.voice.as_deref()).collect();
        assert_eq!(
            voices,
            [
                Some("provider:phonon:2A2fFf8SkDEJ8nzx"),
                // Tippfehler in der Kennung: unbekannt → Standardstimme.
                None,
                Some("provider:phonon:AAAA"),
                Some("provider:phonon:AAAA"),
                // Unbekannter Marker: gestrichen, Standardstimme.
                None,
            ]
        );
        assert!(segs.iter().all(|s| !s.text.contains('<')), "{segs:?}");
        // Das alte Zeilenformat gilt ebenfalls.
        let alt = split_speaker_segments("Anton: Zeile eins.\nNele: Zeile zwei.", &known);
        assert_eq!(alt[0].voice.as_deref(), Some("provider:phonon:AAAA"));
        assert_eq!(
            alt[1].voice.as_deref(),
            Some("provider:phonon:2A2fFf8SkDEJ8nzx")
        );
    }

    fn speaker(id: &str, name: &str) -> ProviderSpeaker {
        ProviderSpeaker {
            id: id.to_string(),
            name: name.to_string(),
            aliases: Vec::new(),
            gender: None,
            language: "de".to_string(),
        }
    }

    #[test]
    fn die_gehaltene_liste_gilt_bis_zum_ablauf_und_nur_fuer_ihre_konfiguration() {
        let t0 = Instant::now();
        let cache =
            RosterCache::after_read(None, "phonon", "k1", Some(vec![speaker("a", "Nele")]), t0);
        assert_eq!(cache.ttl, ROSTER_TTL);
        assert!(cache.is_fresh("phonon", "k1", t0 + Duration::from_secs(59)));
        assert!(
            !cache.is_fresh("phonon", "k1", t0 + ROSTER_TTL),
            "abgelaufen"
        );
        assert!(!cache.is_fresh("phonon", "k2", t0), "andere Konfiguration");
        assert!(
            !cache.is_fresh("openai-speech", "k1", t0),
            "anderer Anbieter"
        );
    }

    #[test]
    fn ein_gescheitertes_lesen_behaelt_die_letzte_liste_nur_derselben_konfiguration() {
        let t0 = Instant::now();
        let good =
            RosterCache::after_read(None, "phonon", "k1", Some(vec![speaker("a", "Nele")]), t0);
        // Gleiche Konfiguration: die alte Liste bleibt, der Wiederversuch kommt bald.
        let kept = RosterCache::after_read(Some(&good), "phonon", "k1", None, t0);
        assert_eq!(kept.speakers.len(), 1);
        assert_eq!(kept.ttl, ROSTER_RETRY);
        assert!(!kept.is_fresh("phonon", "k1", t0 + ROSTER_RETRY));
        // Andere Konfiguration oder anderer Anbieter: nichts wird fremd uebernommen.
        assert!(
            RosterCache::after_read(Some(&good), "phonon", "k2", None, t0)
                .speakers
                .is_empty()
        );
        assert!(
            RosterCache::after_read(Some(&good), "andere", "k1", None, t0)
                .speakers
                .is_empty()
        );
        // Ohne Vorgaenger: leer, aber der Auftrag laeuft weiter.
        assert!(RosterCache::after_read(None, "phonon", "k1", None, t0)
            .speakers
            .is_empty());
    }

    #[test]
    fn die_herkunftsnotiz_nennt_den_namen_des_sprechers() {
        let list = [speaker("nele-id", "Nele"), speaker("anton-id", "Anton")];
        let key = speaker_key("phonon", "anton-id");
        assert_eq!(name_of_key("phonon", &list, &key), Some("Anton"));
        assert_eq!(name_of_key("andere", &list, &key), None, "fremder Anbieter");
        assert_eq!(
            name_of_key("phonon", &list, &speaker_key("phonon", "weg")),
            None,
            "nicht mehr in der Liste"
        );
        assert_eq!(name_of_key("phonon", &list, "erzaehlerin"), None);
    }

    #[test]
    fn eine_gewaehlte_stimme_ist_nicht_leer_und_nicht_default() {
        assert!(voice_is_chosen(Some(&"Nele".to_string())));
        assert!(!voice_is_chosen(None));
        assert!(!voice_is_chosen(Some(&String::new())));
        assert!(!voice_is_chosen(Some(&"  ".to_string())));
        assert!(!voice_is_chosen(Some(&"default".to_string())));
    }

    #[test]
    fn der_schluessel_trennt_die_anbieter_und_die_fish_welt() {
        let key = speaker_key("phonon", "Nele:x");
        assert_eq!(key, "provider:phonon:Nele:x");
        assert_eq!(voice_of_key("phonon", &key), Some("Nele:x"));
        assert_eq!(
            voice_of_key("openai-speech", &key),
            None,
            "fremder Anbieter"
        );
        assert_eq!(voice_of_key("phonon", "erzaehlerin"), None, "Fish-Kennung");
        assert_eq!(
            voice_of_key("phonon", "provider:phonon:"),
            None,
            "leere Stimme"
        );
        // `phonon` darf nicht `phonon2` treffen.
        assert_eq!(voice_of_key("phonon", "provider:phonon2:x"), None);
        assert!(is_provider_key(&key) && !is_provider_key("erzaehlerin"));
    }
}
