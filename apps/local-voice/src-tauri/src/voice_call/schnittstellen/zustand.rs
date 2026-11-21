//! Bindungen der IPC-Runden; beendete Gespraeche bleiben fuer die TTL gesperrt.
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use once_cell::sync::Lazy;

use super::abbruch::{Abbrueche, Zugriffsart};
use super::aeusserung::{Beginn, Rueckfrage};
use super::anbindung::Schnappschuss;
use super::bestaetigung::{Freigabe, MonotoneUhr, Uhr};
use super::gespraeche::Gespraeche;
use super::ipc::{CallResult, Decision, InterfaceError, UtteranceInput};
use super::kodierung::{lies_json, Wert};
use super::umgebung::{Aufrufumgebung, VerweigerndeUmgebung};

#[derive(Clone)]
struct Aufruf {
    antwort: String,
    modell: String,
    name: String,
    argumente: String,
    eintraege: Vec<Wert>,
    ids: Vec<String>,
    frage: Option<(Arc<Freigabe>, u64, String, i64)>,
    ergebnis: Option<Result<CallResult, InterfaceError>>,
}

struct Gespraechsfrage {
    freigabe: Arc<Freigabe>,
    frage: Rueckfrage,
    antwort: String,
    modell: String,
    anfaenge: HashMap<String, Option<Beginn>>,
    frage_ids: HashSet<String>,
    aeusserung_ids: HashSet<String>,
}

const MAX_BINDUNGEN: usize = 256;
const MAX_FRAGEN: usize = 256;
const MAX_AEUSSERUNGEN: usize = 256;
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
    lebenszeichen: Mutex<HashMap<String, Arc<()>>>,
    beendet: Mutex<HashMap<String, i64>>,
    fragen: Mutex<HashMap<String, Gespraechsfrage>>,
    aufrufe: Mutex<HashMap<String, HashMap<String, Aufruf>>>,
    umgebung: Arc<dyn Aufrufumgebung>,
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

#[cfg(test)]
#[path = "tests_aufruf.rs"]
mod tests_aufruf;

impl InterfaceState {
    pub(crate) fn mit_uhr(uhr: Arc<dyn Uhr>) -> Self {
        Self::mit_umgebung(uhr, Arc::new(VerweigerndeUmgebung))
    }

    pub(crate) fn mit_umgebung(uhr: Arc<dyn Uhr>, umgebung: Arc<dyn Aufrufumgebung>) -> Self {
        Self {
            abbrueche: Abbrueche::mit_uhr(Arc::clone(&uhr)),
            gespraeche: Gespraeche::default(),
            bindungen: Mutex::new(HashMap::new()),
            lebenszeichen: Mutex::new(HashMap::new()),
            beendet: Mutex::new(HashMap::new()),
            fragen: Mutex::new(HashMap::new()),
            aufrufe: Mutex::new(HashMap::new()),
            umgebung,
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
        self.lebenszeichen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(c.to_owned())
            .or_insert_with(|| Arc::new(()));
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
        self.lebenszeichen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(c);
        bindungen.retain(|(conversation_id, _), _| conversation_id != c);
        let frage = self
            .fragen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(c);
        if let Some(frage) = frage {
            frage.frage.beende(&frage.freigabe);
        }
        self.gespraeche.ende(c);
        self.aufrufe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(c);
        Ok(())
    }

    pub(crate) fn ist_gebunden(&self, c: &str, a: &str, m: &str) -> bool {
        self.bindungen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(c.to_owned(), a.to_owned()))
            .is_some_and(|bisher| bisher == m)
    }

    pub(crate) fn eroeffne_rueckfrage(
        &self,
        c: &str,
        a: &str,
        m: &str,
        bindung: &str,
        question_id: &str,
    ) -> Result<(), InterfaceError> {
        pruefe_kennungen(&[c, a, m, question_id])?;
        let beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        if beendet.contains_key(c) {
            return Err(InterfaceError::Beendet);
        }
        self.eroeffne_rueckfrage_gesperrt(c, a, m, bindung, question_id)
    }

    fn eroeffne_rueckfrage_gesperrt(
        &self,
        c: &str,
        a: &str,
        m: &str,
        bindung: &str,
        question_id: &str,
    ) -> Result<(), InterfaceError> {
        if !self.ist_gebunden(c, a, m) {
            return Err(InterfaceError::Widerspruch);
        }
        let mut fragen = self.fragen.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(alt) = fragen.get(c) {
            if alt.frage_ids.contains(question_id) {
                return Err(InterfaceError::Widerspruch);
            }
            if alt.frage_ids.len() >= MAX_FRAGEN {
                return Err(InterfaceError::ZuViele);
            }
        }
        let freigabe = fragen.get(c).map_or_else(
            || Arc::new(Freigabe::mit_uhr(Arc::clone(&self.uhr))),
            |alt| Arc::clone(&alt.freigabe),
        );
        let frage = Rueckfrage::neu(Arc::clone(&freigabe), bindung, c, question_id)
            .map_err(|_| InterfaceError::KennungUngueltig)?;
        if let Some(alt) = fragen.get_mut(c) {
            // Kennungen bleiben bis end verbraucht; nur die offene Frage wird ersetzt.
            alt.frage = frage;
            alt.antwort = a.into();
            alt.modell = m.into();
            alt.anfaenge.clear();
            alt.frage_ids.insert(question_id.into());
            return Ok(());
        }
        fragen.insert(
            c.into(),
            Gespraechsfrage {
                freigabe,
                frage,
                antwort: a.into(),
                modell: m.into(),
                anfaenge: HashMap::new(),
                frage_ids: HashSet::from([question_id.into()]),
                aeusserung_ids: HashSet::new(),
            },
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn utterance(
        &self,
        c: &str,
        a: &str,
        m: &str,
        q: &str,
        u: &str,
        input: UtteranceInput,
    ) -> Result<Decision, InterfaceError> {
        // Vor Kennungspruefung und Sperren begrenzen; Text wird nie im Zustand gespeichert.
        // Zu grosse Texte sofort freigeben, den zugehoerigen Beginn aber weiter verbrauchen.
        let input = match input {
            UtteranceInput::Final { text } => {
                if text.chars().take(201).count() > 200 {
                    drop(text);
                    None
                } else {
                    Some(UtteranceInput::Final { text })
                }
            }
            UtteranceInput::Started => Some(UtteranceInput::Started),
        };
        pruefe_kennungen(&[c, a, m, q, u])?;
        // Dieselbe Sperrfolge wie end; darunter gibt es ausschliesslich lokale Arbeit.
        let beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        if beendet.contains_key(c) {
            return Err(InterfaceError::Beendet);
        }
        if !self.ist_gebunden(c, a, m) {
            return Err(InterfaceError::Widerspruch);
        }
        let mut fragen = self.fragen.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(f) = fragen.get_mut(c) else {
            return Ok(Decision::Ungueltig);
        };
        if f.antwort != a || f.modell != m || f.frage.ereignis().question_id != q {
            return Ok(Decision::Ungueltig);
        }
        Ok(match input {
            Some(UtteranceInput::Started) => {
                if f.anfaenge.len() >= 32
                    || f.aeusserung_ids.len() >= MAX_AEUSSERUNGEN
                    || f.aeusserung_ids.contains(u)
                {
                    return Ok(Decision::Ungueltig);
                }
                f.aeusserung_ids.insert(u.into());
                f.anfaenge
                    .insert(u.into(), Some(f.frage.beginne_aeusserung(&f.freigabe)));
                Decision::Wartet
            }
            input => {
                let Some(beginn) = f.anfaenge.get_mut(u).and_then(Option::take) else {
                    return Ok(Decision::Ungueltig);
                };
                let Some(UtteranceInput::Final { text }) = input else {
                    return Ok(Decision::Ungueltig);
                };
                f.frage
                    .aeussere(&f.freigabe, c, u, &beginn, true, &text, "de")
                    .into()
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call(
        &self,
        c: &str,
        a: &str,
        m: &str,
        call_id: &str,
        name: &str,
        argumente: &str,
    ) -> Result<CallResult, InterfaceError> {
        pruefe_kennungen(&[c, a, m, call_id])?;
        let beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        if beendet.contains_key(c) {
            return Err(InterfaceError::Beendet);
        }
        if !self.ist_gebunden(c, a, m) {
            return Err(InterfaceError::Widerspruch);
        }
        let leben = self
            .lebenszeichen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(c)
            .cloned()
            .ok_or(InterfaceError::Beendet)?;
        let mut runde = self
            .gespraeche
            .beginne(c, a)
            .map_err(|_| InterfaceError::Beendet)?;
        let mut aufrufe = self.aufrufe.lock().unwrap_or_else(PoisonError::into_inner);
        let karte = aufrufe.entry(c.into()).or_default();
        let mut aufruf = if let Some(alt) = karte.get_mut(call_id) {
            if alt.antwort != a || alt.modell != m || alt.name != name || alt.argumente != argumente
            {
                return Err(InterfaceError::Widerspruch);
            }
            match &alt.ergebnis {
                None => return Err(InterfaceError::Widerspruch),
                Some(Ok(CallResult::AwaitingConfirmation { .. })) => {}
                Some(ergebnis) => return ergebnis.clone(),
            }
            alt.ergebnis = None;
            alt.clone()
        } else {
            // Keine Verdraengung: auch alte Kennungen duerfen niemals erneut senden.
            if karte.len() >= 32 {
                return Err(InterfaceError::ZuViele);
            }
            let neu = Aufruf {
                antwort: a.into(),
                modell: m.into(),
                name: name.into(),
                argumente: argumente.into(),
                eintraege: self.umgebung.eintraege(),
                ids: self.umgebung.aktive_ids(),
                frage: None,
                ergebnis: None,
            };
            karte.insert(call_id.into(), neu.clone());
            neu
        };
        drop(aufrufe);
        drop(beendet);
        let ergebnis = self.fuehre_gebunden_aus(c, a, m, call_id, || {
            let beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
            if !self
                .lebenszeichen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(c)
                .is_some_and(|aktuell| Arc::ptr_eq(aktuell, &leben))
            {
                return Err(InterfaceError::Beendet);
            }
            let schnappschuss = super::werkzeug::aufloesen(&aufruf.ids, name)
                .and_then(|id| {
                    aufruf
                        .eintraege
                        .iter()
                        .find(|e| e["id"].text() == Some(id.as_str()))
                })
                .and_then(|e| {
                    lies_json(argumente)
                        .ok()
                        .and_then(|w| Schnappschuss::neu(e, &w, None).ok())
                });
            let schreibend = schnappschuss
                .as_ref()
                .is_some_and(Schnappschuss::schreibend);
            if let Some(s) = schnappschuss.as_ref().filter(|s| s.schreibend()) {
                if aufruf.frage.is_none() {
                    // Fragekennungen sind im Gespraech auch fuer manuell eroeffnete Fragen frei.
                    let q = {
                        let fragen = self.fragen.lock().unwrap_or_else(PoisonError::into_inner);
                        (0..=MAX_FRAGEN)
                            .map(|i| format!("call-{i}"))
                            .find(|q| !fragen.get(c).is_some_and(|f| f.frage_ids.contains(q)))
                            .ok_or(InterfaceError::ZuViele)?
                    };
                    self.eroeffne_rueckfrage_gesperrt(c, a, m, s.bindung(), &q)?;
                    let fragen = self.fragen.lock().unwrap_or_else(PoisonError::into_inner);
                    let f = fragen.get(c).ok_or(InterfaceError::Beendet)?;
                    if f.frage.ereignis().question_id != q {
                        return Err(InterfaceError::Widerspruch);
                    }
                    aufruf.frage = Some((
                        f.freigabe.clone(),
                        f.frage.ereignis().generation,
                        q,
                        self.uhr.jetzt_ms(),
                    ));
                }
                if let Some((f, generation, q, erstellt)) = &aufruf.frage {
                    let fragen = self.fragen.lock().unwrap_or_else(PoisonError::into_inner);
                    if fragen.get(c).is_some_and(|frage| {
                        let e = frage.frage.ereignis();
                        e.question_id == q && matches!(e.state, "wartet" | "rueckfrage")
                    }) && self
                        .uhr
                        .jetzt_ms()
                        .checked_sub(*erstellt)
                        .is_some_and(|alter| {
                            (0..super::bestaetigung::BESTAETIGUNG_MS).contains(&alter)
                        })
                        && f.bereit(*generation, s.bindung()).is_err()
                    {
                        return Ok(CallResult::AwaitingConfirmation {
                            question_id: q.clone(),
                        });
                    }
                }
            }
            drop(beendet);
            let art = if schreibend {
                Zugriffsart::Schreibend
            } else {
                Zugriffsart::Lesend
            };
            let handle = self
                .abbrueche
                .registriere(m, art)
                .map_err(|_| InterfaceError::Widerspruch)?;
            let ergebnis = self.umgebung.fuehre(
                &aufruf.eintraege,
                &aufruf.ids,
                name,
                argumente,
                &mut runde,
                handle.abbruch(),
                aufruf.frage.as_ref().map(|(f, g, _, _)| (f.as_ref(), *g)),
            );
            if !ergebnis.protokollzeile.is_empty() {
                log::info!("{}", ergebnis.protokollzeile);
            }
            Ok(ergebnis.into())
        });
        let _beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        if !self
            .lebenszeichen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(c)
            .is_some_and(|aktuell| Arc::ptr_eq(aktuell, &leben))
        {
            return Err(InterfaceError::Beendet);
        }
        aufruf.ergebnis = Some(ergebnis.clone());
        self.aufrufe
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(c.into())
            .or_default()
            .insert(call_id.into(), aufruf);
        ergebnis
    }

    fn fuehre_gebunden_aus(
        &self,
        c: &str,
        a: &str,
        m: &str,
        call_id: &str,
        ausfuehren: impl FnOnce() -> Result<CallResult, InterfaceError>,
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
        let lebenszeichen = self
            .lebenszeichen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(c)
            .cloned()
            .ok_or(InterfaceError::Beendet)?;
        // Die Ausfuehrung darf blockieren; dabei bleibt keine Zustandssperre gehalten.
        drop(beendet);
        let ergebnis = ausfuehren();
        let _beendet = self.beendet.lock().unwrap_or_else(PoisonError::into_inner);
        let lebenszeichen_aktuell = self
            .lebenszeichen
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Auch nach TTL und erneutem begin gehoert das Ergebnis zum alten Gespraech.
        if !lebenszeichen_aktuell
            .get(c)
            .is_some_and(|aktuell| Arc::ptr_eq(aktuell, &lebenszeichen))
        {
            return Err(InterfaceError::Beendet);
        }
        ergebnis
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
