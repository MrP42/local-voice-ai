//! Reine Abbruchregistratur; Reinsprechen unterbricht nur lesende Transporte.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use super::aeusserung::Rueckfrage;
use super::anbindung::Abbruch;
use super::bestaetigung::{Freigabe, MonotoneUhr, Uhr};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Zugriffsart {
    Lesend,
    Schreibend,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Wirkung {
    Abgebrochen,
    Weiterlaufend,
    Vorgemerkt,
    Ungueltig,
}
#[derive(Default)]
struct Zustand {
    laufend: HashMap<String, (Zugriffsart, Abbruch)>,
    vorgemerkt: VecDeque<(String, i64)>,
}
pub(crate) struct Abbrueche {
    zustand: Mutex<Zustand>,
    uhr: Arc<dyn Uhr>,
}
impl Default for Abbrueche {
    fn default() -> Self {
        Self::mit_uhr(Arc::new(MonotoneUhr::default()))
    }
}
impl Abbrueche {
    pub(crate) fn mit_uhr(uhr: Arc<dyn Uhr>) -> Self {
        Self {
            zustand: Mutex::new(Zustand::default()),
            uhr,
        }
    }
    pub(crate) fn registriere(
        &self,
        request_id: &str,
        art: Zugriffsart,
    ) -> Result<Aufrufabbruch<'_>, &'static str> {
        if !kennung_gueltig(request_id) {
            return Err("kennung_ungueltig");
        }
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        if z.laufend.contains_key(request_id) {
            return Err("laeuft_schon");
        }
        if z.laufend.len() >= 64 {
            return Err("zu_viele");
        }
        entferne_abgelaufene(&mut z, self.uhr.jetzt_ms());
        let abbruch = Abbruch::neu();
        if let Some(i) = z.vorgemerkt.iter().position(|(id, _)| id == request_id) {
            z.vorgemerkt.remove(i);
            if art == Zugriffsart::Lesend {
                abbruch.ausloesen();
            }
        }
        z.laufend.insert(request_id.into(), (art, abbruch.clone()));
        Ok(Aufrufabbruch {
            register: self,
            request_id: request_id.into(),
            abbruch,
        })
    }
    pub(crate) fn melde_sprachbeginn(&self, request_id: &str) -> Wirkung {
        if !kennung_gueltig(request_id) {
            return Wirkung::Ungueltig;
        }
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((art, abbruch)) = z.laufend.get(request_id) {
            return if *art == Zugriffsart::Lesend {
                abbruch.ausloesen();
                Wirkung::Abgebrochen
            } else {
                Wirkung::Weiterlaufend
            };
        }
        let jetzt = self.uhr.jetzt_ms();
        entferne_abgelaufene(&mut z, jetzt);
        // Wiederholungen erneuern weder Frist noch Reihenfolge.
        if !z.vorgemerkt.iter().any(|(id, _)| id == request_id) {
            if z.vorgemerkt.len() >= 32 {
                z.vorgemerkt.pop_front();
            }
            z.vorgemerkt.push_back((request_id.into(), jetzt));
        }
        Wirkung::Vorgemerkt
    }
    /// Ausdrueckliches Nein entwertet die Freigabe atomar gegen deren Versand.
    /// Ein bereits begonnener Versand kann dadurch nicht zurueckgenommen werden.
    pub(crate) fn lehne_ab(&self, rueckfrage: &Rueckfrage<'_>, freigabe: &Freigabe) {
        rueckfrage.beende(freigabe);
    }
}
pub(crate) struct Aufrufabbruch<'a> {
    register: &'a Abbrueche,
    request_id: String,
    abbruch: Abbruch,
}
impl Aufrufabbruch<'_> {
    pub(crate) fn abbruch(&self) -> &Abbruch {
        &self.abbruch
    }
}
impl Drop for Aufrufabbruch<'_> {
    fn drop(&mut self) {
        self.register
            .zustand
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .laufend
            .remove(&self.request_id);
    }
}
fn entferne_abgelaufene(z: &mut Zustand, jetzt: i64) {
    z.vorgemerkt.retain(|(_, erstellt)| {
        jetzt
            .checked_sub(*erstellt)
            .is_some_and(|d| (0..30_000).contains(&d))
    });
}
fn kennung_gueltig(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'-'))
}
