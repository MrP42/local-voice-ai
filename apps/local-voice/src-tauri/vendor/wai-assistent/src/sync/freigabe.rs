//! Sync-Freigabe (E3.6) und Upload-Regeln (E3.5, E10).
//!
//! Vor der ausdruecklichen Freigabe geht nichts an den Hub: kein Beitrag, kein Kopf, keine Lease, keine
//! Loeschmarke. Auch das Lesen ist gesperrt. Standard ist aus.

use serde::{Deserialize, Serialize};

use super::verlauf::Beitrag;

/// Schalter "Assistent-Sync" (Standard aus).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncFreigabe {
    aktiv: bool,
}

impl SyncFreigabe {
    pub fn ist_aktiv(&self) -> bool {
        self.aktiv
    }

    /// Ausdrueckliches Einschalten durch den Nutzer.
    pub fn einschalten(&mut self) {
        self.aktiv = true;
    }

    pub fn ausschalten(&mut self) {
        self.aktiv = false;
    }
}

/// Einschraenkungen je Unterhaltung (E3.5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Einschraenkung {
    /// Kein Cloud-Modell, kein Cloud-Werkzeug; Hub-Sync bleibt erlaubt.
    pub modell_lokal: bool,
    /// Kein Hub-Sync dieser Unterhaltung.
    pub geraet_lokal: bool,
}

/// Grund, warum ein Beitrag nicht hochgeladen wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verbot {
    NichtFreigegeben,
    GeraetLokal,
    /// `datenklasse: confidential` in einer `geraet_lokal`-Unterhaltung (E10).
    VertraulichGeraetLokal,
}

/// Darf der Beitrag an den Hub? Einzige Stelle dieser Entscheidung.
pub fn hochladen_erlaubt(
    freigabe: &SyncFreigabe,
    einschraenkung: &Einschraenkung,
    beitrag: &Beitrag,
) -> Result<(), Verbot> {
    if !freigabe.ist_aktiv() {
        return Err(Verbot::NichtFreigegeben);
    }
    match (
        einschraenkung.geraet_lokal,
        beitrag.datenklasse.ist_vertraulich(),
    ) {
        (true, true) => Err(Verbot::VertraulichGeraetLokal),
        (true, false) => Err(Verbot::GeraetLokal),
        (false, _) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::verlauf::{Datenklasse, Modus};

    fn beitrag(klasse: Datenklasse) -> Beitrag {
        let mut b = Beitrag::neu("a", None, 1, "x", Modus::Typed, "t");
        b.datenklasse = klasse;
        b
    }

    #[test]
    fn standard_ist_aus() {
        assert!(!SyncFreigabe::default().ist_aktiv());
        assert_eq!(
            hochladen_erlaubt(
                &SyncFreigabe::default(),
                &Einschraenkung::default(),
                &beitrag(Datenklasse::Public)
            ),
            Err(Verbot::NichtFreigegeben)
        );
    }

    #[test]
    fn geraet_lokal_und_vertraulich_werden_nie_hochgeladen() {
        let mut f = SyncFreigabe::default();
        f.einschalten();
        let lokal = Einschraenkung {
            geraet_lokal: true,
            modell_lokal: false,
        };
        assert_eq!(
            hochladen_erlaubt(&f, &lokal, &beitrag(Datenklasse::Confidential)),
            Err(Verbot::VertraulichGeraetLokal)
        );
        assert_eq!(
            hochladen_erlaubt(&f, &lokal, &beitrag(Datenklasse::Restricted)),
            Err(Verbot::VertraulichGeraetLokal)
        );
        assert_eq!(
            hochladen_erlaubt(&f, &lokal, &beitrag(Datenklasse::Public)),
            Err(Verbot::GeraetLokal)
        );
    }

    #[test]
    fn modell_lokal_erlaubt_sync() {
        let mut f = SyncFreigabe::default();
        f.einschalten();
        let e = Einschraenkung {
            modell_lokal: true,
            geraet_lokal: false,
        };
        assert!(hochladen_erlaubt(&f, &e, &beitrag(Datenklasse::Internal)).is_ok());
        f.ausschalten();
        assert!(hochladen_erlaubt(&f, &e, &beitrag(Datenklasse::Internal)).is_err());
    }
}
