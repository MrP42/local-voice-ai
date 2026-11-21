//! Reiner Automat und Zuordnung menschlicher Aeusserungen zu einer Frage.
//! Der Freigabe-Mutex wird nur beim Entscheiden und beim atomaren Absenden gehalten,
//! nie waehrend DNS/TLS. So kann ein Nein auch waehrend des Verbindungsaufbaus wirken.
//! Die Freigabe hat genau eine Zeitquelle; Aufrufer koennen keine Zeit mitgeben.
use std::sync::{Arc, Mutex, PoisonError};

pub const BESTAETIGUNG_MS: i64 = 30_000;

#[derive(Default)]
pub struct Bestaetigung {
    frage: Option<Frage>,
}
struct Frage {
    bindung: String,
    erstellt: i64,
    bestaetigt: bool,
}
fn gueltig(erstellt: i64, t: i64) -> bool {
    t.checked_sub(erstellt)
        .is_some_and(|d| (0..BESTAETIGUNG_MS).contains(&d))
}
impl Bestaetigung {
    pub fn frage(&mut self, bindung: &str, t: i64) -> &'static str {
        self.frage = Some(Frage {
            bindung: bindung.into(),
            erstellt: t,
            bestaetigt: false,
        });
        "offen"
    }
    pub fn ja(&mut self, t: i64) -> &'static str {
        let Some(f) = &mut self.frage else {
            return "keine_frage";
        };
        if f.bestaetigt {
            return "keine_frage";
        }
        if gueltig(f.erstellt, t) {
            f.bestaetigt = true;
            "bestaetigt"
        } else {
            self.frage = None;
            "abgelaufen"
        }
    }
    pub fn nein(&mut self, _t: i64) -> &'static str {
        if self.frage.take().is_some() {
            "abgelehnt"
        } else {
            "keine_frage"
        }
    }
    pub fn einloesen(&mut self, bindung: &str, t: i64) -> &'static str {
        let Some(f) = &self.frage else {
            return "keine_frage";
        };
        if !f.bestaetigt {
            return "keine_frage";
        }
        let erg = if f.bindung != bindung {
            "bindung_falsch"
        } else if !gueltig(f.erstellt, t) {
            "abgelaufen"
        } else {
            "ok"
        };
        self.frage = None;
        erg
    }
}

pub trait Uhr: Send + Sync {
    fn jetzt_ms(&self) -> i64;
}

/// Monotone Zeit, nie die Wanduhr: Systemzeitspruenge duerfen die
/// 30-Sekunden-Frist weder verlaengern noch verkuerzen.
pub struct MonotoneUhr {
    start: std::time::Instant,
}
impl Default for MonotoneUhr {
    fn default() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }
}
impl Uhr for MonotoneUhr {
    fn jetzt_ms(&self) -> i64 {
        i64::try_from(self.start.elapsed().as_millis()).unwrap_or(i64::MAX)
    }
}

#[derive(Clone)]
pub struct Aeusserung {
    frage: Option<u64>,
    war_offen: bool,
}
#[derive(Default)]
struct Zustand {
    automat: Bestaetigung,
    generation: u64,
    abgelehnt: Option<u64>,
}
pub struct Freigabe {
    zustand: Mutex<Zustand>,
    uhr: Arc<dyn Uhr>,
}
impl Default for Freigabe {
    fn default() -> Self {
        Self::mit_uhr(Arc::new(MonotoneUhr::default()))
    }
}
impl Freigabe {
    pub fn mit_uhr(uhr: Arc<dyn Uhr>) -> Self {
        Self {
            zustand: Mutex::new(Zustand::default()),
            uhr,
        }
    }

    #[cfg(test)]
    pub(super) fn entscheidung_gesperrt(&self) -> bool {
        self.zustand.try_lock().is_err()
    }

    pub fn frage(&self, bindung: &str) -> u64 {
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        let t = self.uhr.jetzt_ms();
        z.generation = z
            .generation
            .checked_add(1)
            .expect("Fragekennungen erschoepft");
        z.abgelehnt = None;
        z.automat.frage(bindung, t);
        z.generation
    }
    pub fn aeusserungsbeginn(&self) -> Aeusserung {
        let z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        Aeusserung {
            frage: z.automat.frage.as_ref().map(|_| z.generation),
            war_offen: z.automat.frage.as_ref().is_some_and(|f| !f.bestaetigt),
        }
    }
    pub fn aeusserung(&self, a: Aeusserung, ja: bool) -> &'static str {
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        let t = self.uhr.jetzt_ms();
        if a.frage != Some(z.generation) || (ja && !a.war_offen) {
            return "keine_frage";
        }
        if ja {
            z.automat.ja(t)
        } else {
            let erg = z.automat.nein(t);
            if erg == "abgelehnt" {
                z.abgelehnt = Some(z.generation);
            }
            erg
        }
    }
    /// Vor DNS/TLS muss bereits dieselbe Frage bestaetigt sein. Diese Vorpruefung
    /// verbraucht nichts; unmittelbar vor Versand wird unter Sperre erneut geprueft.
    pub fn bereit(&self, generation: u64, bindung: &str) -> Result<(), &'static str> {
        let z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        let t = self.uhr.jetzt_ms();
        if z.abgelehnt == Some(generation) {
            return Err("bestaetigung_abgelehnt");
        }
        if generation == z.generation
            && z.automat
                .frage
                .as_ref()
                .is_some_and(|f| f.bestaetigt && f.bindung == bindung && gueltig(f.erstellt, t))
        {
            Ok(())
        } else {
            Err("bestaetigung_abgelaufen")
        }
    }
    /// `senden` muss unmittelbar den ersten Versand auf der fertigen Verbindung beginnen.
    /// Nicht blockierend: kein await, kein DNS/TLS, keine Rueckfrage und kein
    /// Wiedereintritt in Freigabe; auch kein Warten auf Schreibbereitschaft/Antwort.
    /// Sperre und FnOnce machen Einloesen und Versand gegen Entscheidungen unteilbar.
    pub fn absenden<T>(
        &self,
        generation: u64,
        bindung: &str,
        senden: impl FnOnce() -> T,
    ) -> Result<T, &'static str> {
        let mut z = self.zustand.lock().unwrap_or_else(PoisonError::into_inner);
        let t = self.uhr.jetzt_ms();
        if z.abgelehnt == Some(generation) {
            return Err("bestaetigung_abgelehnt");
        }
        if generation != z.generation {
            return Err("bestaetigung_abgelaufen");
        }
        if z.automat.einloesen(bindung, t) != "ok" {
            return Err("bestaetigung_abgelaufen");
        }
        let erg = senden();
        drop(z);
        Ok(erg)
    }
}
