//! Bindungen der IPC-Runden; beendete Gespraeche bleiben dauerhaft gesperrt.
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, PoisonError};

use super::gespraeche::Gespraeche;
use super::ipc::{CallResult, InterfaceError};

const MAX_BINDUNGEN: usize = 256;

#[derive(Default)]
pub(crate) struct InterfaceState {
    gespraeche: Gespraeche,
    bindungen: Mutex<HashMap<(String, String), String>>,
    beendet: Mutex<HashSet<String>>,
}

impl InterfaceState {
    pub(crate) fn begin(&self, c: &str, a: &str, m: &str) -> Result<(), InterfaceError> {
        pruefe_kennungen(&[c, a, m])?;
        // Immer zuerst beendet sperren: Abschluss und Anlage bilden eine Transaktion.
        // Darunter erfolgen ausschliesslich lokale Zugriffe, keine Netzwartezeiten.
        let beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        if beendet.contains(c) {
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
        beendet.insert(c.to_owned());
        self.bindungen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|(conversation_id, _), _| conversation_id != c);
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
        let beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        if beendet.contains(c) {
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
