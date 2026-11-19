use serde::{Deserialize, Serialize};

use super::ZeitGuete;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bereich {
    pub von_ms: u64,
    pub bis_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Zustand {
    Fuellend,
    Bereit,
    PausiertWegenPuffer,
    Fortgesetzt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Grund {
    GrobeZeitmarken,
    Ueberlaenge,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EreignisArt {
    Fehler {
        code: FehlerCode,
    },
    Bereitschaft(Bereich),
    Zustandswechsel(Zustand),
    Satz {
        satz_id: String,
        zeiten: Bereich,
        guete: ZeitGuete,
    },
    SynchronNichtVerfuegbar {
        satz_id: String,
        zeiten: Bereich,
        grund: Grund,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FehlerCode {
    KeinLokalesModell,
    Quelle,
    Uebersetzung,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ereignis {
    pub session_id: String,
    pub epoch: u64,
    pub art: EreignisArt,
}

/// Auch andere Produzenten können denselben Epochenfilter verwenden.
pub fn aktuell(
    session_id: &str,
    epoch: u64,
    ergebnis_epoch: u64,
    art: EreignisArt,
) -> Option<Ereignis> {
    (epoch == ergebnis_epoch).then(|| Ereignis {
        session_id: session_id.to_owned(),
        epoch,
        art,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alte_epoche_bleibt_stumm() {
        // Gegenprobe: Ohne Epochenfilter würde ein veraltetes Ereignis erscheinen.
        let art = EreignisArt::Zustandswechsel(Zustand::Bereit);
        assert!(aktuell("s", 2, 1, art.clone()).is_none());
        assert_eq!(aktuell("s", 2, 2, art).map(|e| e.epoch), Some(2));
    }
}
