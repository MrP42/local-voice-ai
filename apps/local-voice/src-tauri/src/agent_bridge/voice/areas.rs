//! Wissensbereiche des Sprachgespraechs: was das Sprachmodell nachschlagen darf.
//!
//! Der Nutzer gibt Bereiche frei (Einstellung `voice_call_knowledge`, Standard leer). Diese
//! Freigabe ist getrennt von den Rechten externer Agenten an der Agentenbruecke: sie entscheidet
//! allein, welche Werkzeuge das Gespraech bekommt (`voice::list`) und ausfuehren darf
//! (`voice::call`). Jeder Zugriff laeuft trotzdem durch das Tor mit Audit.
//!
//! # Der Bereich „Transkripte“
//! Er ist der bestehende Unterschalter `voice_call_read_transcripts` (eine Quelle der Wahrheit,
//! kein zweiter Eintrag in der Liste) und gilt nur zusammen mit „Besprechungen“: ohne
//! Besprechungen gibt es nichts, wozu ein Transkript gehoert. Ein Schluessel `transcripts` in der
//! Liste wird ueberlesen.
//!
//! Dieses Modul ist rein (keine Datenbank, keine Uhr, keine App).

/// Ein Wissensbereich.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Area {
    Meetings,
    Transcripts,
    Dictations,
    Conversations,
    Calendar,
    Tts,
}

impl Area {
    pub const ALL: [Area; 6] = [
        Area::Meetings,
        Area::Transcripts,
        Area::Dictations,
        Area::Conversations,
        Area::Calendar,
        Area::Tts,
    ];

    /// Schluessel in der Einstellung und im Frontend.
    pub fn key(self) -> &'static str {
        match self {
            Area::Meetings => "meetings",
            Area::Transcripts => "transcripts",
            Area::Dictations => "dictations",
            Area::Conversations => "conversations",
            Area::Calendar => "calendar",
            Area::Tts => "tts",
        }
    }

    pub fn parse(key: &str) -> Option<Area> {
        Area::ALL.into_iter().find(|a| a.key() == key)
    }

    /// Name fuer Meldungen an das Modell (und damit an den Nutzer).
    pub fn label(self) -> &'static str {
        match self {
            Area::Meetings => "Besprechungen",
            Area::Transcripts => "Transkripte",
            Area::Dictations => "Diktate",
            Area::Conversations => "frühere Gespräche",
            Area::Calendar => "Termine",
            Area::Tts => "Vorlesen-Seiten",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// Die freigegebenen Bereiche (klein, kopierbar).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Areas {
    bits: u8,
}

impl Areas {
    /// Nichts freigegeben (der Standard).
    pub fn none() -> Self {
        Self::default()
    }

    /// Aus der Einstellung: `keys` ist `voice_call_knowledge`, `read_transcripts` der Unterschalter
    /// `voice_call_read_transcripts`. Unbekannte Schluessel zaehlen nicht (fail closed).
    pub fn from_settings(keys: &[String], read_transcripts: bool) -> Self {
        let mut bits = 0u8;
        for key in keys {
            if let Some(area) = Area::parse(key) {
                if area != Area::Transcripts {
                    bits |= area.bit();
                }
            }
        }
        let mut areas = Areas { bits };
        if read_transcripts && areas.has(Area::Meetings) {
            areas.bits |= Area::Transcripts.bit();
        }
        areas
    }

    pub fn has(self, area: Area) -> bool {
        self.bits & area.bit() != 0
    }

    pub fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// Mindestens einer der Bereiche ist frei.
    pub fn has_any(self, areas: &[Area]) -> bool {
        areas.iter().any(|a| self.has(*a))
    }

    /// Die freien Bereiche in fester Reihenfolge.
    pub fn granted(self) -> Vec<Area> {
        Area::ALL.into_iter().filter(|a| self.has(*a)).collect()
    }

    /// Die fehlenden Bereiche in fester Reihenfolge. „Transkripte“ fehlen nicht eigens, solange
    /// die Besprechungen fehlen (es gaebe ohnehin nichts zu lesen).
    pub fn missing(self) -> Vec<Area> {
        Area::ALL
            .into_iter()
            .filter(|a| !self.has(*a))
            .filter(|a| *a != Area::Transcripts || self.has(Area::Meetings))
            .collect()
    }
}

/// Die Liste fuer `voice_call_knowledge` beim Speichern: nur bekannte Bereiche ausser
/// „Transkripte“ (die haben ihren Unterschalter), ohne Doppelte, in fester Reihenfolge.
pub fn normalize_keys(keys: &[String]) -> Vec<String> {
    Area::ALL
        .into_iter()
        .filter(|a| *a != Area::Transcripts)
        .filter(|a| keys.iter().any(|k| k == a.key()))
        .map(|a| a.key().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn nothing_is_shared_by_default() {
        let a = Areas::from_settings(&[], false);
        assert!(a.is_empty());
        assert_eq!(a, Areas::none());
        assert!(a.granted().is_empty());
        assert_eq!(a.missing().len(), 5, "Transkripte fehlen nicht eigens");
        // Der Unterschalter allein gibt nichts frei.
        assert!(Areas::from_settings(&[], true).is_empty());
    }

    #[test]
    fn transcripts_need_meetings_and_the_sub_switch() {
        let m = Areas::from_settings(&keys(&["meetings"]), false);
        assert!(m.has(Area::Meetings) && !m.has(Area::Transcripts));
        assert!(m.missing().contains(&Area::Transcripts));
        let mt = Areas::from_settings(&keys(&["meetings"]), true);
        assert!(mt.has(Area::Transcripts));
        let t = Areas::from_settings(&keys(&["dictations"]), true);
        assert!(
            !t.has(Area::Transcripts),
            "ohne Besprechungen kein Transkript"
        );
        // Ein Schluessel `transcripts` in der Liste gibt nichts frei.
        let l = Areas::from_settings(&keys(&["meetings", "transcripts"]), false);
        assert!(!l.has(Area::Transcripts));
    }

    #[test]
    fn unknown_keys_do_not_count() {
        let a = Areas::from_settings(&keys(&["", "Meetings", "alles", "calendar"]), false);
        assert_eq!(a.granted(), vec![Area::Calendar]);
    }

    #[test]
    fn keys_round_trip_and_labels_exist() {
        for area in Area::ALL {
            assert_eq!(Area::parse(area.key()), Some(area));
            assert!(!area.label().is_empty());
        }
        assert_eq!(Area::parse("x"), None);
        let all = Areas::from_settings(
            &keys(&["tts", "calendar", "conversations", "dictations", "meetings"]),
            true,
        );
        assert_eq!(all.granted().len(), 6);
        assert!(all.missing().is_empty());
    }

    #[test]
    fn saving_keeps_only_known_areas_once_in_order() {
        let out = normalize_keys(&keys(&[
            "tts",
            "meetings",
            "meetings",
            "transcripts",
            "unsinn",
            "calendar",
        ]));
        assert_eq!(out, keys(&["meetings", "calendar", "tts"]));
        assert!(normalize_keys(&[]).is_empty());
    }
}
