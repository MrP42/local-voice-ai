//! Budget eines Laufs (Schema `Budget`) mit Zaehler. Ueberschreitung ist ein Fehler, nie ein stilles Weiter.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::fehler::RechercheFehler;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub searches: u64,
    pub fetches: u64,
    pub tokens: u64,
    pub seconds: u64,
}

#[derive(Debug)]
pub struct BudgetZaehler {
    grenze: Budget,
    searches: u64,
    fetches: u64,
    tokens: u64,
    start: Instant,
}

impl BudgetZaehler {
    pub fn neu(grenze: Budget) -> Self {
        Self {
            grenze,
            searches: 0,
            fetches: 0,
            tokens: 0,
            start: Instant::now(),
        }
    }

    /// Setzt einen angehaltenen Lauf fort: bisher Verbrauchtes (inkl. Sekunden) zaehlt weiter.
    pub fn mit_verbrauch(grenze: Budget, verbraucht: Budget) -> Self {
        let start = Instant::now()
            .checked_sub(std::time::Duration::from_secs(verbraucht.seconds))
            .unwrap_or_else(Instant::now);
        Self {
            grenze,
            searches: verbraucht.searches,
            fetches: verbraucht.fetches,
            tokens: verbraucht.tokens,
            start,
        }
    }

    pub fn grenze(&self) -> Budget {
        self.grenze
    }

    /// Verbrauch (`budget_used`); Sekunden werden aus der Uhr gelesen.
    pub fn verbraucht(&self) -> Budget {
        Budget {
            searches: self.searches,
            fetches: self.fetches,
            tokens: self.tokens,
            seconds: self.start.elapsed().as_secs(),
        }
    }

    /// Restzeit bis zur Sekundengrenze (praezise); deckelt Zeitlimits einzelner Abrufe.
    pub fn verbleibend(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.grenze.seconds).saturating_sub(self.start.elapsed())
    }

    pub fn zeit_pruefen(&self) -> Result<(), RechercheFehler> {
        if self.start.elapsed().as_secs() >= self.grenze.seconds {
            return Err(RechercheFehler::BudgetErreicht("seconds"));
        }
        Ok(())
    }

    /// Verbucht eine Suche; vorher pruefen, damit die Grenze nie ueberschritten wird.
    pub fn suche(&mut self) -> Result<(), RechercheFehler> {
        self.zeit_pruefen()?;
        if self.searches >= self.grenze.searches {
            return Err(RechercheFehler::BudgetErreicht("searches"));
        }
        self.searches += 1;
        Ok(())
    }

    pub fn abruf(&mut self) -> Result<(), RechercheFehler> {
        self.zeit_pruefen()?;
        if self.fetches >= self.grenze.fetches {
            return Err(RechercheFehler::BudgetErreicht("fetches"));
        }
        self.fetches += 1;
        Ok(())
    }

    pub fn tokens(&mut self, n: u64) -> Result<(), RechercheFehler> {
        self.zeit_pruefen()?;
        if self.tokens.saturating_add(n) > self.grenze.tokens {
            return Err(RechercheFehler::BudgetErreicht("tokens"));
        }
        self.tokens += n;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grenze() -> Budget {
        Budget {
            searches: 2,
            fetches: 1,
            tokens: 100,
            seconds: 60,
        }
    }

    #[test]
    fn suchen_bis_zur_grenze_dann_fehler() {
        let mut z = BudgetZaehler::neu(grenze());
        assert!(z.suche().is_ok());
        assert!(z.suche().is_ok());
        assert!(matches!(
            z.suche(),
            Err(RechercheFehler::BudgetErreicht("searches"))
        ));
        assert_eq!(z.verbraucht().searches, 2);
    }

    #[test]
    fn abrufe_und_tokens() {
        let mut z = BudgetZaehler::neu(grenze());
        assert!(z.abruf().is_ok());
        assert!(matches!(
            z.abruf(),
            Err(RechercheFehler::BudgetErreicht("fetches"))
        ));
        assert!(z.tokens(60).is_ok());
        assert!(matches!(
            z.tokens(41),
            Err(RechercheFehler::BudgetErreicht("tokens"))
        ));
        assert!(z.tokens(40).is_ok());
        assert_eq!(z.verbraucht().tokens, 100);
    }

    #[test]
    fn zeitgrenze_null_ist_sofort_erreicht() {
        let mut z = BudgetZaehler::neu(Budget {
            seconds: 0,
            ..grenze()
        });
        assert!(matches!(
            z.suche(),
            Err(RechercheFehler::BudgetErreicht("seconds"))
        ));
    }

    #[test]
    fn mit_verbrauch_uebernimmt_zaehler_und_zeit() {
        let v = Budget {
            searches: 2,
            fetches: 0,
            tokens: 90,
            seconds: 59,
        };
        let mut z = BudgetZaehler::mit_verbrauch(grenze(), v);
        assert!(matches!(
            z.suche(),
            Err(RechercheFehler::BudgetErreicht("searches"))
        ));
        assert!(matches!(
            z.tokens(11),
            Err(RechercheFehler::BudgetErreicht("tokens"))
        ));
        assert!(z.verbraucht().seconds >= 59);
        let spaet = BudgetZaehler::mit_verbrauch(grenze(), Budget { seconds: 60, ..v });
        assert!(spaet.zeit_pruefen().is_err());
    }

    #[test]
    fn serde_feldnamen() {
        let j = serde_json::to_value(grenze()).unwrap();
        assert_eq!(j["searches"], 2);
        assert_eq!(j["seconds"], 60);
    }
}
