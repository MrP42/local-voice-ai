//! Reiner Automat und Zuordnung menschlicher Aeusserungen zu einer Frage.
//! Der Freigabe-Mutex wird nur beim Entscheiden und beim atomaren Absenden gehalten,
//! nie waehrend DNS/TLS. So kann ein Nein auch waehrend des Verbindungsaufbaus wirken.
use std::sync::Mutex;

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

pub trait Uhr {
    fn jetzt_ms(&self) -> i64;
}

#[derive(Clone, Copy)]
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
#[derive(Default)]
pub struct Freigabe {
    zustand: Mutex<Zustand>,
}
impl Freigabe {
    #[cfg(test)]
    pub(super) fn entscheidung_gesperrt(&self) -> bool {
        self.zustand.try_lock().is_err()
    }

    pub fn frage(&self, bindung: &str, t: i64) -> u64 {
        let mut z = self.zustand.lock().expect("Freigabe-Mutex");
        z.generation = z
            .generation
            .checked_add(1)
            .expect("Fragekennungen erschoepft");
        z.abgelehnt = None;
        z.automat.frage(bindung, t);
        z.generation
    }
    pub fn aeusserungsbeginn(&self) -> Aeusserung {
        let z = self.zustand.lock().expect("Freigabe-Mutex");
        Aeusserung {
            frage: z.automat.frage.as_ref().map(|_| z.generation),
            war_offen: z.automat.frage.as_ref().is_some_and(|f| !f.bestaetigt),
        }
    }
    pub fn aeusserung(&self, a: Aeusserung, ja: bool, t: i64) -> &'static str {
        let mut z = self.zustand.lock().expect("Freigabe-Mutex");
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
    pub fn bereit(
        &self,
        generation: u64,
        bindung: &str,
        uhr: &impl Uhr,
    ) -> Result<(), &'static str> {
        let z = self.zustand.lock().expect("Freigabe-Mutex");
        if z.abgelehnt == Some(generation) {
            return Err("bestaetigung_abgelehnt");
        }
        if generation == z.generation
            && z.automat.frage.as_ref().is_some_and(|f| {
                f.bestaetigt && f.bindung == bindung && gueltig(f.erstellt, uhr.jetzt_ms())
            })
        {
            Ok(())
        } else {
            Err("bestaetigung_abgelaufen")
        }
    }
    /// `senden` muss unmittelbar den ersten Versand auf der fertigen Verbindung beginnen.
    /// Kein await, kein DNS/TLS, keine Rueckfrage und kein Wiedereintritt in Freigabe.
    /// Sperre und FnOnce machen Einloesen und Versand gegen Entscheidungen unteilbar.
    pub fn absenden<T>(
        &self,
        generation: u64,
        bindung: &str,
        uhr: &impl Uhr,
        senden: impl FnOnce() -> T,
    ) -> Result<T, &'static str> {
        let mut z = self.zustand.lock().expect("Freigabe-Mutex");
        if z.abgelehnt == Some(generation) {
            return Err("bestaetigung_abgelehnt");
        }
        if generation != z.generation {
            return Err("bestaetigung_abgelaufen");
        }
        if z.automat.einloesen(bindung, uhr.jetzt_ms()) != "ok" {
            return Err("bestaetigung_abgelaufen");
        }
        let erg = senden();
        drop(z);
        Ok(erg)
    }
}
