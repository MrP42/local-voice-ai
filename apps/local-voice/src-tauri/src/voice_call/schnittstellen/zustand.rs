//! Bindungen der IPC-Runden; beendete Gespraeche bleiben fuer die TTL gesperrt.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use once_cell::sync::Lazy;

use super::abbruch::Abbrueche;
use super::bestaetigung::{MonotoneUhr, Uhr};
use super::gespraeche::Gespraeche;
use super::ipc::{CallResult, InterfaceError};

const MAX_BINDUNGEN: usize = 256;
const MAX_BEENDET: usize = 4096;
const BEENDET_TTL_MS: i64 = 24 * 60 * 60 * 1000;

static GEMEINSAM: Lazy<Arc<InterfaceState>> = Lazy::new(|| Arc::new(InterfaceState::default()));

pub(crate) fn gemeinsam() -> Arc<InterfaceState> {
    Arc::clone(&GEMEINSAM)
}

pub(crate) struct InterfaceState {
    // Der Cancel-Hook meldet auch Widerrufe; er unterscheidet die Herkunft nicht.
    pub(crate) abbrueche: Abbrueche,
    gespraeche: Gespraeche,
    bindungen: Mutex<HashMap<(String, String), String>>,
    beendet: Mutex<HashMap<String, i64>>,
    uhr: Arc<dyn Uhr>,
}

impl Default for InterfaceState {
    fn default() -> Self {
        Self::mit_uhr(Arc::new(MonotoneUhr::default()))
    }
}

#[cfg(test)]
#[path = "tests_zustand.rs"]
mod tests;

impl InterfaceState {
    pub(crate) fn mit_uhr(uhr: Arc<dyn Uhr>) -> Self {
        Self {
            abbrueche: Abbrueche::mit_uhr(Arc::clone(&uhr)),
            gespraeche: Gespraeche::default(),
            bindungen: Mutex::new(HashMap::new()),
            beendet: Mutex::new(HashMap::new()),
            uhr,
        }
    }

    fn entferne_abgelaufene(&self, beendet: &mut HashMap<String, i64>) {
        let jetzt = self.uhr.jetzt_ms();
        // Rueckspruenge und Ueberlauf duerfen die Wiederbelebungssperre nicht loesen.
        beendet.retain(|_, erstellt| {
            !jetzt
                .checked_sub(*erstellt)
                .is_some_and(|alter| alter >= BEENDET_TTL_MS)
        });
    }

    pub(crate) fn begin(&self, c: &str, a: &str, m: &str) -> Result<(), InterfaceError> {
        pruefe_kennungen(&[c, a, m])?;
        // Immer zuerst beendet sperren: Abschluss und Anlage bilden eine Transaktion.
        // Darunter erfolgen ausschliesslich lokale Zugriffe, keine Netzwartezeiten.
        let mut beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        self.entferne_abgelaufene(&mut beendet);
        if beendet.contains_key(c) {
            return Err(InterfaceError::Beendet);
        }
        let mut bindungen = self
            .bindungen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let schluessel = (c.to_owned(), a.to_owned());
        if let Some(bisher) = bindungen.get(&schluessel) {
            return if bisher == m {
                Ok(())
            } else {
                Err(InterfaceError::Widerspruch)
            };
        }
        if bindungen.values().any(|bisher| bisher == m) {
            return Err(InterfaceError::Widerspruch);
        }
        if bindungen.len() >= MAX_BINDUNGEN {
            return Err(InterfaceError::ZuViele);
        }
        // Laufende Gespraeche reservieren ihren Abschlussplatz schon bei begin.
        // So kann end die Grenze einhalten, ohne eine bestehende Sperre zu verdraengen.
        let bekannt: HashSet<_> = bindungen.keys().map(|(id, _)| id.as_str()).collect();
        if !bekannt.contains(c) && beendet.len() + bekannt.len() >= MAX_BEENDET {
            return Err(InterfaceError::ZuViele);
        }
        self.gespraeche.beginne(c, a).map_err(|code| match code {
            "kennung_ungueltig" => InterfaceError::KennungUngueltig,
            "gespraech_beendet" => InterfaceError::Beendet,
            _ => InterfaceError::ZuViele,
        })?;
        bindungen.insert(schluessel, m.to_owned());
        Ok(())
    }

    pub(crate) fn end(&self, c: &str) -> Result<(), InterfaceError> {
        pruefe_kennungen(&[c])?;
        let mut beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        self.entferne_abgelaufene(&mut beendet);
        let mut bindungen = self
            .bindungen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if !bindungen.keys().any(|(id, _)| id == c) {
            return Ok(());
        }
        beendet.insert(c.to_owned(), self.uhr.jetzt_ms());
        bindungen.retain(|(conversation_id, _), _| conversation_id != c);
        self.gespraeche.ende(c);
        Ok(())
    }

    pub(crate) fn ist_gebunden(&self, c: &str, a: &str, m: &str) -> bool {
        self.bindungen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(c.to_owned(), a.to_owned()))
            .is_some_and(|bisher| bisher == m)
    }

    pub(crate) fn call(
        &self,
        c: &str,
        a: &str,
        m: &str,
        call_id: &str,
    ) -> Result<CallResult, InterfaceError> {
        pruefe_kennungen(&[c, a, m, call_id])?;
        let mut beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        self.entferne_abgelaufene(&mut beendet);
        if beendet.contains_key(c) {
            return Err(InterfaceError::Beendet);
        }
        if !self.ist_gebunden(c, a, m) {
            return Err(InterfaceError::Widerspruch);
        }
        // Paket B2.6a hat absichtlich keinen Zugang zu Adaptern oder Transporten.
        Err(InterfaceError::Verweigert)
    }
}

fn pruefe_kennungen(kennungen: &[&str]) -> Result<(), InterfaceError> {
    if kennungen.iter().all(|id| {
        (1..=64).contains(&id.len())
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'-'))
    }) {
        Ok(())
    } else {
        Err(InterfaceError::KennungUngueltig)
    }
}
