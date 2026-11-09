//! Testhilfe: ein fertig gekoppeltes Konto mit mehreren Geraeten, ohne den Kopplungsablauf durchzuspielen.
//!
//! Das erste Geraet ist der Vertrauensanker und nimmt alle weiteren per signiertem Verzeichniseintrag auf;
//! alle Geraete teilen Kontosalz und Epochenschluessel 1. Nur fuer Tests; in Produktion laeuft die
//! Kopplung ueber [`super::kopplung::Kopplung`] und [`super::abgleich::Abgleich::geraet_aufnehmen`].

use std::collections::BTreeMap;

use super::abgleich::Abgleich;
use super::hub::{AuthGeraet, HubSpeicher};
use super::identitaet::Geraeteschluessel;
use super::memhub::MemHub;
use super::schluessel::Schluesselbund;
use super::speicher::MemSpeicher;
use super::verzeichnis::Verzeichnis;

/// Gekoppeltes Testkonto.
pub struct Testkonto {
    geraete: BTreeMap<String, Geraeteschluessel>,
    bund: Schluesselbund,
    verzeichnis: Verzeichnis,
}

impl Testkonto {
    /// Konto mit den genannten Geraeten; das erste ist der Anker.
    ///
    /// # Panics
    /// Bei leerer Liste oder doppelten Namen (Testfehler).
    pub fn neu(namen: &[&str]) -> Self {
        let (anker, rest) = namen.split_first().expect("mindestens ein Geraet");
        let anker_id = Geraeteschluessel::erzeugen(anker).expect("schluessel");
        let mut verzeichnis = Verzeichnis::gruenden(&anker_id);
        let mut geraete = BTreeMap::new();
        for n in rest {
            let g = Geraeteschluessel::erzeugen(n).expect("schluessel");
            let e = verzeichnis
                .aufnahme_vorschlagen(g.ausweis(), &anker_id)
                .expect("aufnahme");
            assert!(verzeichnis.anhaengen(e).expect("anhaengen"));
            geraete.insert((*n).to_owned(), g);
        }
        geraete.insert((*anker).to_owned(), anker_id);
        Self {
            geraete,
            bund: Schluesselbund::zufaellig().expect("bund"),
            verzeichnis,
        }
    }

    /// Wie [`Testkonto::neu`] und legt das Verzeichnis beim Hub ab.
    pub fn mit_hub(namen: &[&str], hub: &MemHub) -> Self {
        let k = Self::neu(namen);
        let anker = AuthGeraet::new(namen[0]);
        for e in k.verzeichnis.eintraege() {
            hub.verzeichnis_anhaengen(&anker, e)
                .expect("hub verzeichnis");
        }
        k
    }

    /// Geraet mit eigenem Arbeitsspeicher-Zustandsspeicher, Sync freigegeben.
    pub fn geraet(&self, name: &str) -> Abgleich {
        self.geraet_mit_speicher(name, MemSpeicher::new())
    }

    /// Geraet mit gegebenem Speicher (Klone teilen den Block: Neustart per
    /// [`Abgleich::wiederherstellen`]).
    pub fn geraet_mit_speicher(&self, name: &str, speicher: MemSpeicher) -> Abgleich {
        let mut a = Abgleich::gekoppelt(
            self.schluessel(name).clone(),
            self.bund.clone(),
            self.verzeichnis.clone(),
            Box::new(speicher),
        )
        .expect("gekoppelt");
        a.freigabe_einschalten().expect("freigabe");
        a
    }

    pub fn bund(&self) -> &Schluesselbund {
        &self.bund
    }

    pub fn verzeichnis(&self) -> &Verzeichnis {
        &self.verzeichnis
    }

    /// Geraeteschluessel (Identitaet) eines Geraets.
    ///
    /// # Panics
    /// Bei unbekanntem Namen (Testfehler).
    pub fn schluessel(&self, name: &str) -> &Geraeteschluessel {
        self.geraete.get(name).expect("unbekanntes Testgeraet")
    }
}
