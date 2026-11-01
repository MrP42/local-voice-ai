//! Rückstau und Vorrang (Codex-Auflage 2: „Audio darf Steuerbefehle nicht blockieren“).
//!
//! - **Guthaben:** Audio fließt nur, solange der Empfänger dafür Guthaben gewährt hat (Rahmen je
//!   Strom). Für Audio hinein gewährt der Dienst ([`grenzen::GUTHABEN_START`] mit `guthaben`
//!   nach `start`), für Audio heraus die App. Beide Seiten führen eine [`Guthaben`]-Instanz je
//!   Richtung: der Sender ruft [`Guthaben::verbrauchen`] vor dem Senden, der Empfänger beim
//!   Empfang. Kommt Audio ohne Guthaben an, endet die Verbindung mit `kein_guthaben`.
//! - **Ausgang:** eine Warteschlange je Verbindung mit drei Spuren: Abbrüche (FIFO, immer zuerst),
//!   Steuerung, Audio. Ein Abbruch verwirft wartendes Audio und wartende Steuerrahmen (`teil`,
//!   `ergebnis`) desselben Stroms. Steuerung ist auf [`STEUER_MAX`] Rahmen begrenzt; nach
//!   [`FAIR`] Steuerrahmen in Folge geht ein Audiorahmen, damit eine Flut von Steuerung Audio
//!   nicht aushungert. Audio ist in Bytes begrenzt ([`grenzen::AUDIO_PUFFER_MAX`]); ist der
//!   Puffer voll, wird der neue Rahmen abgewiesen und der Aufrufer meldet Überlauf.

use std::collections::{HashMap, VecDeque};

use crate::frame::{Art, Rahmen};
use crate::grenzen;

/// Höchstzahl wartender Steuerrahmen; darüber endet die Verbindung (`ueberlastet`).
pub const STEUER_MAX: usize = 64;
/// Steuerrahmen in Folge, bevor ein wartender Audiorahmen dran ist.
pub const FAIR: usize = 8;

/// Guthaben einer Richtung je Strom.
#[derive(Debug, Default)]
pub struct Guthaben {
    je_strom: HashMap<u32, u32>,
}

impl Guthaben {
    /// Strom öffnen; `start` ist das gewährte Anfangsguthaben (0, wenn erst `guthaben` folgt).
    pub fn oeffnen(&mut self, strom: u32, start: u32) {
        self.je_strom
            .insert(strom, start.min(grenzen::GUTHABEN_MAX));
    }

    pub fn schliessen(&mut self, strom: u32) {
        self.je_strom.remove(&strom);
    }

    /// Mehr Guthaben; gedeckelt auf [`grenzen::GUTHABEN_MAX`] insgesamt.
    pub fn gewaehren(&mut self, strom: u32, rahmen: u32) {
        if let Some(g) = self.je_strom.get_mut(&strom) {
            *g = g.saturating_add(rahmen).min(grenzen::GUTHABEN_MAX);
        }
    }

    /// Verbraucht einen Rahmen; `false` heißt: nicht senden bzw. Protokollfehler beim Empfang.
    pub fn verbrauchen(&mut self, strom: u32) -> bool {
        match self.je_strom.get_mut(&strom) {
            Some(g) if *g > 0 => {
                *g -= 1;
                true
            }
            _ => false,
        }
    }

    pub fn stand(&self, strom: u32) -> u32 {
        self.je_strom.get(&strom).copied().unwrap_or(0)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Einreihen {
    Ok,
    /// Puffer voll: Rahmen nicht angenommen (Audio: Überlauf melden; Steuerung: Verbindung beenden).
    Voll,
}

/// Ausgangswarteschlange mit Vorrang für Abbrüche und Steuerrahmen.
#[derive(Debug, Default)]
pub struct Ausgang {
    abbrueche: VecDeque<Rahmen>,
    steuer: VecDeque<Rahmen>,
    audio: VecDeque<Rahmen>,
    audio_bytes: usize,
    steuer_in_folge: usize,
}

impl Ausgang {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn steuer(&mut self, r: Rahmen) -> Einreihen {
        debug_assert_eq!(r.art, Art::Steuer);
        if self.steuer.len() >= STEUER_MAX {
            return Einreihen::Voll;
        }
        self.steuer.push_back(r);
        Einreihen::Ok
    }

    /// Abbruch: vor alle anderen Rahmen (Abbrüche untereinander in Reihenfolge), wartendes Audio
    /// und wartende Steuerrahmen desselben Stroms verwerfen.
    pub fn abbrechen(&mut self, strom: u32, r: Rahmen) {
        debug_assert_eq!(r.art, Art::Steuer);
        self.audio.retain(|a| a.strom != strom);
        self.audio_bytes = self.audio.iter().map(|a| a.nutzlast.len()).sum();
        self.steuer.retain(|s| s.strom != strom);
        self.abbrueche.push_back(r);
    }

    pub fn audio(&mut self, r: Rahmen) -> Einreihen {
        debug_assert_ne!(r.art, Art::Steuer);
        if self.audio_bytes + r.nutzlast.len() > grenzen::AUDIO_PUFFER_MAX {
            return Einreihen::Voll;
        }
        self.audio_bytes += r.nutzlast.len();
        self.audio.push_back(r);
        Einreihen::Ok
    }

    /// Nächster zu sendender Rahmen.
    pub fn naechster(&mut self) -> Option<Rahmen> {
        if let Some(r) = self.abbrueche.pop_front() {
            return Some(r);
        }
        let audio_dran = self.steuer_in_folge >= FAIR && !self.audio.is_empty();
        if !audio_dran && let Some(r) = self.steuer.pop_front() {
            self.steuer_in_folge += 1;
            return Some(r);
        }
        let r = self.audio.pop_front()?;
        self.steuer_in_folge = 0;
        self.audio_bytes -= r.nutzlast.len();
        Some(r)
    }

    pub fn audio_bytes(&self) -> usize {
        self.audio_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(strom: u32, n: usize) -> Rahmen {
        Rahmen {
            art: Art::AudioHinein,
            strom,
            nutzlast: vec![0; n],
        }
    }

    #[test]
    fn guthaben_begrenzt_und_deckelt() {
        let mut g = Guthaben::default();
        assert!(!g.verbrauchen(1), "ohne Öffnen kein Senden");
        g.oeffnen(1, grenzen::GUTHABEN_START);
        for _ in 0..grenzen::GUTHABEN_START {
            assert!(g.verbrauchen(1));
        }
        assert!(!g.verbrauchen(1));
        g.gewaehren(1, 1_000);
        assert_eq!(g.stand(1), grenzen::GUTHABEN_MAX);
        g.oeffnen(2, 0);
        assert!(!g.verbrauchen(2), "Sprechen: erst Guthaben der App");
        g.schliessen(1);
        g.gewaehren(1, 5);
        assert_eq!(g.stand(1), 0, "geschlossener Strom bekommt kein Guthaben");
    }

    #[test]
    fn abbrechen_ueberholt_und_raeumt_den_strom() {
        let mut a = Ausgang::new();
        a.audio(audio(1, 10));
        a.audio(audio(2, 10));
        a.steuer(Rahmen::steuer(1, b"teil".to_vec()));
        a.steuer(Rahmen::steuer(0, b"s".to_vec()));
        a.abbrechen(1, Rahmen::steuer(1, b"x1".to_vec()));
        a.abbrechen(2, Rahmen::steuer(2, b"x2".to_vec()));
        assert_eq!(a.naechster().unwrap().nutzlast, b"x1");
        assert_eq!(
            a.naechster().unwrap().nutzlast,
            b"x2",
            "Abbrüche in Reihenfolge"
        );
        assert_eq!(
            a.naechster().unwrap().nutzlast,
            b"s",
            "teil von Strom 1 verworfen"
        );
        assert!(a.naechster().is_none(), "Audio beider Ströme verworfen");
        assert_eq!(a.audio_bytes(), 0);
    }

    #[test]
    fn steuerflut_hungert_audio_nicht_aus_und_ist_begrenzt() {
        let mut a = Ausgang::new();
        a.audio(audio(1, 10));
        let mut n = 0;
        while a.steuer(Rahmen::steuer(0, b"s".to_vec())) == Einreihen::Ok {
            n += 1;
        }
        assert_eq!(n, STEUER_MAX);
        for _ in 0..FAIR {
            assert_eq!(a.naechster().unwrap().art, Art::Steuer);
        }
        assert_eq!(a.naechster().unwrap().art, Art::AudioHinein);
    }

    #[test]
    fn audio_puffer_ist_begrenzt() {
        let mut a = Ausgang::new();
        let mut n = 0;
        while a.audio(audio(1, grenzen::AUDIO_MAX)) == Einreihen::Ok {
            n += 1;
        }
        assert_eq!(n, grenzen::AUDIO_PUFFER_MAX / grenzen::AUDIO_MAX);
        a.steuer(Rahmen::steuer(1, b"ende".to_vec()));
        assert_eq!(
            a.naechster().unwrap().art,
            Art::Steuer,
            "voller Audiopuffer blockiert Steuerung nicht"
        );
    }
}
