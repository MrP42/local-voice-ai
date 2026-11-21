//! Registratur der Gespraeche und ihrer Antwortrunden ohne Netzwartezeiten.
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use super::anbindung::{Antwortrunde, Gespraech};

const MAX_GESPRAECHE: usize = 64;
const MAX_ANTWORTEN: usize = 8;

#[derive(Default)]
pub(crate) struct Gespraeche {
    karte: Mutex<HashMap<String, Eintrag>>,
    #[cfg(test)]
    test_barriere: Mutex<Option<std::sync::Arc<std::sync::Barrier>>>,
}

#[derive(Default)]
struct Eintrag {
    gespraech: Gespraech,
    beendet: bool,
    antworten: VecDeque<(String, Antwortrunde)>,
}

impl Gespraeche {
    pub(crate) fn neu() -> Self {
        Self::default()
    }

    pub(crate) fn beginne(
        &self,
        conversation_id: &str,
        answer_id: &str,
    ) -> Result<Antwortrunde, &'static str> {
        if !kennung_gueltig(conversation_id) || !kennung_gueltig(answer_id) {
            return Err("kennung_ungueltig");
        }
        let mut karte = self
            .karte
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        entferne_beendete(&mut karte);
        if !karte.contains_key(conversation_id) && karte.len() >= MAX_GESPRAECHE {
            return Err("zu_viele_gespraeche");
        }
        let eintrag = karte.entry(conversation_id.into()).or_default();
        if eintrag.beendet {
            return Err("gespraech_beendet");
        }
        if let Some((_, runde)) = eintrag.antworten.iter().find(|(id, _)| id == answer_id) {
            return Ok(runde.clone());
        }
        if eintrag.antworten.len() >= MAX_ANTWORTEN {
            let index = eintrag
                .antworten
                .iter()
                .position(|(_, runde)| {
                    if runde.laeuft() {
                        return false;
                    }
                    #[cfg(test)]
                    self.warte_fuer_test();
                    runde.verwerfen()
                })
                .ok_or("zu_viele_antworten")?;
            eintrag.antworten.remove(index);
        }
        // Alle Antworten desselben Gespraechs muessen dessen Sperre teilen.
        let runde = eintrag.gespraech.antwortrunde();
        eintrag
            .antworten
            .push_back((answer_id.into(), runde.clone()));
        Ok(runde)
    }

    pub(crate) fn ende(&self, conversation_id: &str) {
        let mut karte = self
            .karte
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        if let Some(eintrag) = karte.get_mut(conversation_id) {
            eintrag.gespraech.beenden();
            eintrag.beendet = true;
            #[cfg(test)]
            self.warte_fuer_test();
        }
        entferne_beendete(&mut karte);
    }

    pub(crate) fn aufraeumen(&self) {
        let mut karte = self
            .karte
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        entferne_beendete(&mut karte);
    }

    pub(crate) fn anzahl(&self) -> usize {
        self.karte
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner())
            .len()
    }

    #[cfg(test)]
    pub(super) fn setze_barriere_fuer_test(&self, barriere: std::sync::Arc<std::sync::Barrier>) {
        *self.test_barriere.lock().unwrap() = Some(barriere);
    }

    #[cfg(test)]
    fn warte_fuer_test(&self) {
        // Nur einmal warten, damit weitere Verdraengungskandidaten frei pruefbar bleiben.
        let barriere = self.test_barriere.lock().unwrap().take();
        if let Some(barriere) = barriere {
            barriere.wait();
            barriere.wait();
        }
    }

    #[cfg(test)]
    pub(super) fn vergifte_karte_fuer_test(&self) {
        let _karte = self
            .karte
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        panic!("Testpanik unter dem Karten-Mutex");
    }
}

fn entferne_beendete(karte: &mut HashMap<String, Eintrag>) {
    karte.retain(|_, eintrag| !eintrag.beendet || eintrag.gespraech.besetzt());
}

fn kennung_gueltig(kennung: &str) -> bool {
    (1..=64).contains(&kennung.len())
        && kennung.bytes().all(|zeichen| {
            zeichen.is_ascii_alphanumeric() || matches!(zeichen, b'.' | b'_' | b':' | b'-')
        })
}
