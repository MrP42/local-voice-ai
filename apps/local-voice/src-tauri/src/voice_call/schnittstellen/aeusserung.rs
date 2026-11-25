//! Lokale Auswertung gesprochener Bestaetigungen ohne Netz oder Inhaltsprotokoll.
use super::bestaetigung::{Aeusserung, Freigabe, BESTAETIGUNG_MS};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

static RUECKFRAGE_KENNUNG: AtomicU64 = AtomicU64::new(1);
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Urteil {
    Ja,
    Nein,
    Unklar,
}
pub(crate) fn bewerte(text: &str, sprache: &str) -> Urteil {
    if text.chars().take(201).count() > 200 {
        return Urteil::Unklar;
    }
    let text = text.to_lowercase();
    if text.contains("...") {
        return Urteil::Unklar;
    }
    let text = text.trim_matches(|c: char| c.is_whitespace() || satzzeichen(c));
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.split_whitespace().count() > 6 {
        return Urteil::Unklar;
    }
    match text.as_str() {
        "nein" | "nein danke" | "nicht" | "stopp" | "stop" | "abbrechen" | "no" | "no thanks"
        | "cancel" => Urteil::Nein,
        "ja" | "ja bitte" | "mach das" | "ausfuehren" | "ja ausfuehren" | "okay mach"
            if sprache == "de" =>
        {
            Urteil::Ja
        }
        "yes" | "yes please" | "do it" | "go ahead" if sprache == "en" => Urteil::Ja,
        _ => Urteil::Unklar,
    }
}
fn satzzeichen(c: char) -> bool {
    matches!(c, '.' | '!' | ',')
}
fn kennung_gueltig(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'-'))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entscheidung {
    Wartet,
    Rueckfrage,
    Freigegeben,
    Abgelehnt,
    Abgelaufen,
    Ungueltig,
}
struct Zustand {
    entscheidung: Entscheidung,
    ids: Vec<String>,
    beendet: bool,
}
pub(crate) struct Rueckfrage {
    freigabe: Arc<Freigabe>,
    kennung: u64,
    konversation: String,
    question_id: String,
    generation: u64,
    zustand: Mutex<Zustand>,
}
pub(crate) struct Beginn {
    kennung: u64,
    generation: u64,
    aeusserung: Aeusserung,
}
#[derive(Debug)]
pub(crate) struct Fragezustand<'a> {
    pub text_key: &'static str,
    pub question_id: &'a str,
    pub generation: u64,
    pub state: &'static str,
    pub timeout_ms: u64,
}
impl Rueckfrage {
    pub(crate) const RUECKFRAGE: &'static str = "Bitte antworte mit Ja oder Nein.";
    pub(crate) fn neu(
        freigabe: Arc<Freigabe>,
        bindung: &str,
        konversation: &str,
        question_id: &str,
    ) -> Result<Self, &'static str> {
        if !kennung_gueltig(konversation) || !kennung_gueltig(question_id) {
            return Err("kennung_ungueltig");
        }
        Ok(Self {
            kennung: RUECKFRAGE_KENNUNG
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
                .expect("Rueckfragekennungen erschoepft"),
            konversation: konversation.into(),
            question_id: question_id.into(),
            generation: freigabe.frage(bindung),
            freigabe,
            zustand: Mutex::new(Zustand {
                entscheidung: Entscheidung::Wartet,
                ids: Vec::new(),
                beendet: false,
            }),
        })
    }
    pub(crate) fn beginne_aeusserung(&self, freigabe: &Freigabe) -> Beginn {
        Beginn {
            kennung: if std::ptr::eq(self.freigabe.as_ref(), freigabe) {
                self.kennung
            } else {
                0
            },
            generation: self.generation,
            aeusserung: freigabe.aeusserungsbeginn(),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn aeussere(
        &self,
        freigabe: &Freigabe,
        konversation: &str,
        utterance_id: &str,
        beginn: &Beginn,
        finale: bool,
        text: &str,
        sprache: &str,
    ) -> Entscheidung {
        use Entscheidung::*;
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        if z.beendet || !matches!(z.entscheidung, Wartet | Rueckfrage) {
            return Ungueltig;
        }
        if !std::ptr::eq(self.freigabe.as_ref(), freigabe)
            || beginn.kennung != self.kennung
            || beginn.generation != self.generation
            || beginn.aeusserung.frage != Some(self.generation)
            || !beginn.aeusserung.war_offen
            || konversation != self.konversation
            || !kennung_gueltig(utterance_id)
            || z.ids.iter().any(|id| id == utterance_id)
        {
            return Ungueltig;
        }
        if !finale {
            return Wartet;
        }
        if z.ids.len() >= 32 {
            return Ungueltig;
        }
        z.ids.push(utterance_id.into());
        // Nur lesen: insbesondere Unklar darf weder bestaetigen noch die Frist erneuern.
        {
            let f = freigabe
                .zustand
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let offen = f.automat.frage.as_ref();
            if f.generation != self.generation
                || !offen.is_some_and(|frage| {
                    freigabe
                        .uhr
                        .jetzt_ms()
                        .checked_sub(frage.erstellt)
                        .is_some_and(|d| (0..BESTAETIGUNG_MS).contains(&d))
                })
            {
                z.entscheidung = Abgelaufen;
                return Abgelaufen;
            }
        }
        let ja = match bewerte(text, sprache) {
            Urteil::Ja => true,
            Urteil::Nein => false,
            Urteil::Unklar => {
                z.entscheidung = Rueckfrage;
                return Rueckfrage;
            }
        };
        z.entscheidung = match freigabe.aeusserung(beginn.aeusserung.clone(), ja) {
            "bestaetigt" => Freigegeben,
            "abgelehnt" => Abgelehnt,
            "abgelaufen" | "keine_frage" => Abgelaufen,
            _ => Ungueltig,
        };
        z.entscheidung
    }
    pub(crate) fn beende(&self, freigabe: &Freigabe) {
        if !std::ptr::eq(self.freigabe.as_ref(), freigabe) {
            return;
        }
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        freigabe.verwerfen(self.generation);
        z.beendet = true;
        if matches!(
            z.entscheidung,
            Entscheidung::Wartet | Entscheidung::Rueckfrage | Entscheidung::Freigegeben
        ) {
            z.entscheidung = Entscheidung::Abgelaufen;
        }
    }
    pub(crate) fn ereignis(&self) -> Fragezustand<'_> {
        let z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        Fragezustand {
            text_key: if z.entscheidung == Entscheidung::Rueckfrage {
                "bestaetigung_rueckfrage"
            } else {
                "bestaetigung_frage"
            },
            question_id: &self.question_id,
            generation: self.generation,
            state: match z.entscheidung {
                Entscheidung::Wartet | Entscheidung::Ungueltig => "wartet",
                Entscheidung::Rueckfrage => "rueckfrage",
                Entscheidung::Freigegeben => "freigegeben",
                Entscheidung::Abgelehnt => "abgelehnt",
                Entscheidung::Abgelaufen => "abgelaufen",
            },
            timeout_ms: BESTAETIGUNG_MS as u64,
        }
    }
}
/// Ausschliesslich fuer lokale TTS; keine Weitergabe an Cloud-Dienste.
pub(crate) fn vorschau_text(name: &str, schreibend: bool, sprache: &str) -> Option<String> {
    if !(1..=60).contains(&name.chars().count())
        || name.trim().is_empty()
        || name.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '\u{2028}'
                        | '\u{2029}'
                        | '"'
                        | '\''
                        | '„'
                        | '“'
                        | '”'
                        | '‚'
                        | '‘'
                        | '’'
                        | '«'
                        | '»'
                        | '‹'
                        | '›'
                )
        })
    {
        return None;
    }
    let (frage, hinweis) = match sprache {
        "de" => (
            format!("Soll ich {name} ausfuehren?"),
            " Das aendert Daten.",
        ),
        "en" => (format!("Shall I run {name}?"), " This changes data."),
        _ => return None,
    };
    Some(format!("{frage}{}", if schreibend { hinweis } else { "" }))
}
