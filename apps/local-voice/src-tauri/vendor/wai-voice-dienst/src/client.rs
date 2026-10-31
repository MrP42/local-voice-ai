//! Blockierender Client für die App-Seite über einen beliebigen Bytestrom (Pipe, Unix-Socket, in
//! Tests ein Socket-Paar). Prüft jede eingehende Nachricht gegen den Vertrag, hält das Guthaben für
//! Audio hinein und schickt nie mehr, als der Dienst erlaubt hat.
//!
//! Der Client protokolliert keine Inhalte (weder Audio noch Text).

use std::collections::HashSet;
use std::io::{Read, Write};

use thiserror::Error;

use crate::beweis;
use crate::fluss::Guthaben;
use crate::frame::{Art, Decoder, Rahmen, RahmenFehler, Rolle};
use crate::grenzen;
use crate::msg::{
    AnApp, AnDienst, Engines, Operation, Recht, Souveraenitaet, Wortpaar, pruefen, pruefen_an_app,
};

#[derive(Debug, Error)]
pub enum ClientFehler {
    #[error("Verbindung: {0}")]
    Io(#[from] std::io::Error),
    #[error("Rahmen: {0}")]
    Rahmen(#[from] RahmenFehler),
    #[error("Vertrag verletzt: {0}")]
    Vertrag(String),
    #[error("Dienst meldet {code}")]
    Dienst { code: String, nachricht: String },
    #[error("Verbindung beendet")]
    Ende,
}

impl ClientFehler {
    /// Fehlercode für die Oberfläche (Dienstcode oder `verbindung`).
    pub fn code(&self) -> &str {
        match self {
            Self::Dienst { code, .. } => code,
            Self::Vertrag(_) | Self::Rahmen(_) => "vertrag",
            Self::Io(_) | Self::Ende => "verbindung",
        }
    }
}

/// Was nach `willkommen` feststeht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Angemeldet {
    pub dienst: String,
    pub version: String,
    pub rechte: Vec<Recht>,
    pub engines: Engines,
}

/// Ein empfangenes Ereignis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ereignis {
    Steuer(AnApp),
    /// WAV-Häppchen eines `sprechen`-Stroms.
    Audio {
        strom: u32,
        daten: Vec<u8>,
    },
}

pub struct Verbindung<S: Read + Write> {
    leitung: S,
    decoder: Decoder,
    rest: Vec<u8>,
    guthaben: Guthaben,
    /// Offene Ströme; Nachrichten für andere Ströme sind Vertragsverletzungen.
    offen: HashSet<u32>,
    /// Je Verbindung benutzte Stromnummern (keine Wiederverwendung: späte Rahmen eines alten
    /// Stroms dürfen nie einem neuen gutgeschrieben werden).
    benutzt: HashSet<u32>,
    /// `anmelden` oder `koppeln` lief schon (nur einmal je Verbindung).
    begonnen: bool,
}

impl<S: Read + Write> Verbindung<S> {
    pub fn neu(leitung: S) -> Self {
        Self {
            leitung,
            decoder: Decoder::new(Rolle::App),
            rest: Vec::new(),
            guthaben: Guthaben::default(),
            offen: HashSet::new(),
            benutzt: HashSet::new(),
            begonnen: false,
        }
    }

    /// Steuernachricht senden (vorher gegen den Vertrag geprüft).
    pub fn senden(&mut self, m: &AnDienst) -> Result<(), ClientFehler> {
        pruefen(m).map_err(ClientFehler::Vertrag)?;
        let json = serde_json::to_vec(m).map_err(|e| ClientFehler::Vertrag(e.to_string()))?;
        let bytes = Rahmen::steuer(m.strom(), json).kodieren()?;
        self.leitung.write_all(&bytes)?;
        self.leitung.flush()?;
        Ok(())
    }

    /// Einen Audiorahmen (PCM16 LE 16 kHz mono, höchstens 1 s) senden; verbraucht ein Guthaben.
    /// `Ok(false)`: kein Guthaben, erst [`Verbindung::empfangen`] aufrufen.
    pub fn audio(&mut self, strom: u32, pcm: &[u8]) -> Result<bool, ClientFehler> {
        if pcm.len() > grenzen::AUDIO_MAX || !pcm.len().is_multiple_of(2) {
            return Err(ClientFehler::Vertrag("audio".into()));
        }
        if !self.offen.contains(&strom) {
            return Err(ClientFehler::Vertrag("strom".into()));
        }
        if !self.guthaben.verbrauchen(strom) {
            return Ok(false);
        }
        let bytes = Rahmen {
            art: Art::AudioHinein,
            strom,
            nutzlast: pcm.to_vec(),
        }
        .kodieren()?;
        self.leitung.write_all(&bytes)?;
        self.leitung.flush()?;
        Ok(true)
    }

    /// Nächstes Ereignis lesen (blockiert). Guthaben des Dienstes wird hier verbucht.
    pub fn empfangen(&mut self) -> Result<Ereignis, ClientFehler> {
        loop {
            if !self.rest.is_empty() {
                let n = self.decoder.einspeisen(&self.rest)?;
                self.rest.drain(..n);
            }
            if let Some(r) = self.decoder.naechster()? {
                return self.auswerten(r);
            }
            if !self.rest.is_empty() {
                continue;
            }
            let mut buf = [0u8; 8192];
            let n = self.leitung.read(&mut buf)?;
            if n == 0 {
                return Err(ClientFehler::Ende);
            }
            self.rest.extend_from_slice(&buf[..n]);
        }
    }

    fn auswerten(&mut self, r: Rahmen) -> Result<Ereignis, ClientFehler> {
        match r.art {
            // Sprechen bietet dieser Client noch nicht an: Audio heraus ist nie erwartet.
            Art::AudioHeraus => Err(ClientFehler::Vertrag("audio_unerwartet".into())),
            Art::Steuer => {
                let m: AnApp = serde_json::from_slice(&r.nutzlast)
                    .map_err(|_| ClientFehler::Vertrag("json".into()))?;
                pruefen_an_app(&m).map_err(ClientFehler::Vertrag)?;
                if m.strom() != r.strom {
                    return Err(ClientFehler::Vertrag("strom".into()));
                }
                // Nachrichten nur zu Strömen, die diese Verbindung geöffnet hat (Strom 0: Verbindung).
                // Späte Rahmen eines beendeten Stroms (etwa `fehler abgebrochen`) sind erlaubt.
                if m.strom() != 0 && !self.benutzt.contains(&m.strom()) {
                    return Err(ClientFehler::Vertrag("strom_unbekannt".into()));
                }
                if let AnApp::Guthaben { strom, rahmen } = m {
                    self.guthaben.gewaehren(strom, rahmen);
                }
                Ok(Ereignis::Steuer(m))
            }
            Art::AudioHinein => Err(ClientFehler::Vertrag("richtung".into())),
        }
    }

    fn steuer(&mut self) -> Result<AnApp, ClientFehler> {
        match self.empfangen()? {
            Ereignis::Steuer(AnApp::Fehler {
                code, nachricht, ..
            }) => Err(ClientFehler::Dienst { code, nachricht }),
            Ereignis::Steuer(m) => Ok(m),
            Ereignis::Audio { .. } => Err(ClientFehler::Vertrag("audio_unerwartet".into())),
        }
    }

    fn gruss(&mut self) -> Result<String, ClientFehler> {
        if self.begonnen {
            return Err(ClientFehler::Vertrag("zustand".into()));
        }
        self.begonnen = true;
        match self.steuer()? {
            AnApp::Gruss { zufall, .. } => Ok(zufall),
            _ => Err(ClientFehler::Vertrag("gruss".into())),
        }
    }

    /// Handschlag mit Zugang und Schlüssel; der Schlüssel verlässt die App nicht.
    pub fn anmelden(
        &mut self,
        client: &str,
        zugang: &str,
        schluessel: &str,
    ) -> Result<Angemeldet, ClientFehler> {
        let zufall = self.gruss()?;
        let fp = beweis::fingerabdruck(schluessel);
        self.senden(&AnDienst::Hallo {
            protokoll: crate::PROTOKOLL,
            client: client.into(),
            zugang: zugang.into(),
            beweis: beweis::erzeugen(&fp, &zufall, client),
        })?;
        match self.steuer()? {
            AnApp::Willkommen {
                dienst,
                version,
                rechte,
                engines,
                beweis: gegen,
                ..
            } => {
                if !beweis::dienst_pruefen(&fp, &zufall, client, &gegen) {
                    return Err(ClientFehler::Vertrag("dienst_beweis".into()));
                }
                Ok(Angemeldet {
                    dienst,
                    version,
                    rechte,
                    engines,
                })
            }
            _ => Err(ClientFehler::Vertrag("willkommen".into())),
        }
    }

    /// Kopplung anfragen; wartet, bis der Nutzer in Local Voice AI bestätigt. Liefert den Zugang.
    pub fn koppeln(
        &mut self,
        client: &str,
        anzeige: &str,
        schluessel: &str,
        code: &str,
    ) -> Result<String, ClientFehler> {
        self.gruss()?;
        let fp = beweis::fingerabdruck(schluessel);
        self.senden(&AnDienst::Koppeln {
            protokoll: crate::PROTOKOLL,
            client: client.into(),
            anzeige: anzeige.into(),
            fingerabdruck: fp.iter().map(|b| format!("{b:02x}")).collect(),
            code: code.into(),
        })?;
        match self.steuer()? {
            AnApp::Gekoppelt { zugang } => Ok(zugang),
            _ => Err(ClientFehler::Vertrag("gekoppelt".into())),
        }
    }

    /// Strom zum Erkennen öffnen. Jede Stromnummer nur einmal je Verbindung, höchstens
    /// [`grenzen::STROEME_MAX`] zugleich. Das Startguthaben kommt als `guthaben` vom Dienst.
    pub fn erkennen_starten(
        &mut self,
        strom: u32,
        teilergebnisse: bool,
        bereich: &str,
        souveraenitaet: Souveraenitaet,
    ) -> Result<(), ClientFehler> {
        if strom == 0 || self.benutzt.contains(&strom) {
            return Err(ClientFehler::Vertrag("strom_belegt".into()));
        }
        if self.offen.len() >= grenzen::STROEME_MAX {
            return Err(ClientFehler::Vertrag("zu_viele_stroeme".into()));
        }
        let start = AnDienst::Start {
            strom,
            op: if teilergebnisse {
                Operation::ErkennenStrom
            } else {
                Operation::Erkennen
            },
            bereich: bereich.into(),
            souveraenitaet,
        };
        pruefen(&start).map_err(ClientFehler::Vertrag)?;
        self.benutzt.insert(strom);
        self.offen.insert(strom);
        self.guthaben.oeffnen(strom, 0);
        self.senden(&start).inspect_err(|_| self.schliessen(strom))
    }

    fn schliessen(&mut self, strom: u32) {
        self.offen.remove(&strom);
        self.guthaben.schliessen(strom);
    }

    /// Eine ganze Aufnahme erkennen: sendet in Rahmen zu 1 s, sobald Guthaben da ist, dann `ende`,
    /// und wartet auf das Ergebnis. Teilergebnisse gehen an `teil`. Kommt das Ergebnis schon
    /// während der Aufnahme (Höchstdauer), wird es sofort geliefert.
    pub fn erkennen(
        &mut self,
        strom: u32,
        bereich: &str,
        souveraenitaet: Souveraenitaet,
        pcm: &[u8],
        mut teil: impl FnMut(&str),
    ) -> Result<String, ClientFehler> {
        if !pcm.len().is_multiple_of(2) {
            return Err(ClientFehler::Vertrag("pcm".into()));
        }
        self.erkennen_starten(strom, false, bereich, souveraenitaet)?;
        let r = self.erkennen_lauf(strom, pcm, &mut teil);
        self.schliessen(strom);
        r
    }

    fn erkennen_lauf(
        &mut self,
        strom: u32,
        pcm: &[u8],
        teil: &mut impl FnMut(&str),
    ) -> Result<String, ClientFehler> {
        for stueck in pcm.chunks(grenzen::AUDIO_MAX) {
            while !self.audio(strom, stueck)? {
                if let Some(text) = self.warten(strom, teil)? {
                    return Ok(text);
                }
            }
        }
        self.senden(&AnDienst::Ende { strom })?;
        loop {
            if let Some(text) = self.warten(strom, teil)? {
                return Ok(text);
            }
        }
    }

    /// Ein Ereignis verarbeiten: `Some(text)` beim Ergebnis von `strom`. Ein `fehler` auf Strom 0
    /// ist laut Vertrag endgültig für die Verbindung.
    fn warten(
        &mut self,
        strom: u32,
        teil: &mut impl FnMut(&str),
    ) -> Result<Option<String>, ClientFehler> {
        match self.empfangen()? {
            Ereignis::Steuer(AnApp::Ergebnis { strom: s, text }) if s == strom => Ok(Some(text)),
            Ereignis::Steuer(AnApp::Teil { strom: s, text }) if s == strom => {
                teil(&text);
                Ok(None)
            }
            Ereignis::Steuer(AnApp::Fehler {
                strom: s,
                code,
                nachricht,
            }) if s == strom || s == 0 => Err(ClientFehler::Dienst { code, nachricht }),
            Ereignis::Steuer(AnApp::Fehler { strom: s, .. }) => {
                self.schliessen(s);
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    /// Korrektursignal senden (nur mit Recht `lernen` und eingeschaltetem Lernen im Bereich).
    pub fn rueckmelden(&mut self, bereich: &str, paare: Vec<Wortpaar>) -> Result<(), ClientFehler> {
        self.senden(&AnDienst::Rueckmeldung {
            bereich: bereich.into(),
            paare,
        })
    }

    /// Strom sofort beenden.
    pub fn abbrechen(&mut self, strom: u32) -> Result<(), ClientFehler> {
        self.schliessen(strom);
        self.senden(&AnDienst::Abbrechen { strom })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::thread;

    use super::*;
    use crate::msg::{Sitzung, code};

    const ZUFALL: &str = "ab";

    fn zufall() -> String {
        ZUFALL.repeat(32)
    }

    /// Minimaler Dienst für Tests: Handschlag gegen einen Schlüssel, Erkennen liefert die Zahl
    /// der empfangenen Bytes als Text; Guthaben 1 je Rahmen.
    fn dienst(mut s: UnixStream, schluessel: &str, rechte: Vec<Recht>) -> Vec<String> {
        let fp = beweis::fingerabdruck(schluessel);
        let mut d = Decoder::new(Rolle::Dienst);
        let mut sitzung = Sitzung::new();
        let mut log = Vec::new();
        let mut empfangen = 0usize;
        let sende = |s: &mut UnixStream, m: AnApp| {
            let r = Rahmen::steuer(m.strom(), serde_json::to_vec(&m).unwrap());
            s.write_all(&r.kodieren().unwrap()).unwrap();
        };
        sende(
            &mut s,
            AnApp::Gruss {
                protokoll: crate::PROTOKOLL,
                zufall: zufall(),
            },
        );
        let mut buf = [0u8; 4096];
        let mut rest = Vec::new();
        loop {
            while !rest.is_empty() {
                let n = d.einspeisen(&rest).unwrap();
                rest.drain(..n);
                if n == 0 {
                    break;
                }
            }
            let Some(r) = d.naechster().unwrap() else {
                let n = match s.read(&mut buf) {
                    Ok(0) | Err(_) => return log,
                    Ok(n) => n,
                };
                rest.extend_from_slice(&buf[..n]);
                continue;
            };
            if r.art == Art::AudioHinein {
                assert!(sitzung.audio_erlaubt(r.strom));
                empfangen += r.nutzlast.len();
                sende(
                    &mut s,
                    AnApp::Guthaben {
                        strom: r.strom,
                        rahmen: 1,
                    },
                );
                continue;
            }
            let m: AnDienst = serde_json::from_slice(&r.nutzlast).unwrap();
            pruefen(&m).unwrap();
            sitzung.annehmen(r.strom, &m).unwrap();
            log.push(format!("{:?}", std::mem::discriminant(&m)));
            match m {
                AnDienst::Hallo {
                    client, beweis: b, ..
                } => {
                    if beweis::pruefen(&fp, &zufall(), &client, &b) {
                        sitzung.anmelden();
                        sende(
                            &mut s,
                            AnApp::Willkommen {
                                protokoll: crate::PROTOKOLL,
                                dienst: "test".into(),
                                version: "0".into(),
                                rechte: rechte.clone(),
                                engines: Engines {
                                    erkennung: "e".into(),
                                    stimme: "s".into(),
                                    lokal: true,
                                },
                                beweis: beweis::dienst_erzeugen(&fp, &zufall(), &client),
                            },
                        );
                    } else {
                        sende(
                            &mut s,
                            AnApp::Fehler {
                                strom: 0,
                                code: code::SCHLUESSEL_UNGUELTIG.into(),
                                nachricht: "".into(),
                            },
                        );
                        return log;
                    }
                }
                AnDienst::Koppeln {
                    fingerabdruck,
                    code: c,
                    ..
                } => {
                    let soll: String = fp.iter().map(|b| format!("{b:02x}")).collect();
                    if fingerabdruck == soll && c == "123456" {
                        sende(
                            &mut s,
                            AnApp::Gekoppelt {
                                zugang: "z-1".into(),
                            },
                        );
                    } else {
                        sende(
                            &mut s,
                            AnApp::Fehler {
                                strom: 0,
                                code: code::ABGELEHNT.into(),
                                nachricht: "".into(),
                            },
                        );
                    }
                    return log;
                }
                AnDienst::Start { strom, .. } => {
                    if !rechte.contains(&Recht::Hoeren) {
                        sitzung.schliessen(strom);
                        sende(
                            &mut s,
                            AnApp::Fehler {
                                strom,
                                code: code::RECHT_FEHLT.into(),
                                nachricht: "".into(),
                            },
                        );
                        continue;
                    }
                    empfangen = 0;
                    sende(&mut s, AnApp::Guthaben { strom, rahmen: 1 });
                }
                AnDienst::Ende { strom } => {
                    sende(
                        &mut s,
                        AnApp::Teil {
                            strom,
                            text: "…".into(),
                        },
                    );
                    sende(
                        &mut s,
                        AnApp::Ergebnis {
                            strom,
                            text: format!("{empfangen} Bytes"),
                        },
                    );
                    sitzung.schliessen(strom);
                }
                _ => {}
            }
        }
    }

    fn paar(
        schluessel: &'static str,
        rechte: Vec<Recht>,
    ) -> (Verbindung<UnixStream>, thread::JoinHandle<Vec<String>>) {
        let (a, b) = UnixStream::pair().unwrap();
        let h = thread::spawn(move || dienst(b, schluessel, rechte));
        (Verbindung::neu(a), h)
    }

    #[test]
    fn anmelden_und_erkennen_mit_rueckstau() {
        let (mut v, h) = paar("geheim", vec![Recht::Hoeren]);
        let a = v.anmelden("corporate-llm", "z-1", "geheim").unwrap();
        assert_eq!(a.rechte, vec![Recht::Hoeren]);
        // 2,5 s Audio = 3 Rahmen bei Guthaben 1: der Client muss zweimal auf Guthaben warten.
        let pcm = vec![0u8; 80_000];
        let mut teile = Vec::new();
        let text = v
            .erkennen(1, "allgemein", Souveraenitaet::NurLokal, &pcm, |t| {
                teile.push(t.to_owned())
            })
            .unwrap();
        assert_eq!(text, "80000 Bytes");
        assert_eq!(teile, vec!["…"]);
        drop(v);
        h.join().unwrap();
    }

    #[test]
    fn falscher_schluessel_wird_abgewiesen() {
        let (mut v, h) = paar("geheim", vec![Recht::Hoeren]);
        let e = v.anmelden("corporate-llm", "z-1", "falsch").unwrap_err();
        assert_eq!(e.code(), code::SCHLUESSEL_UNGUELTIG);
        h.join().unwrap();
    }

    #[test]
    fn fehlendes_recht_kommt_als_dienstfehler() {
        let (mut v, h) = paar("geheim", vec![]);
        v.anmelden("corporate-llm", "z-1", "geheim").unwrap();
        let e = v
            .erkennen(1, "allgemein", Souveraenitaet::NurLokal, &[0, 0], |_| {})
            .unwrap_err();
        assert_eq!(e.code(), code::RECHT_FEHLT);
        drop(v);
        h.join().unwrap();
    }

    #[test]
    fn koppeln_schickt_nur_den_fingerabdruck() {
        let (mut v, h) = paar("neu-erzeugt", vec![]);
        assert_eq!(
            v.koppeln("corporate-llm", "Corporate LLM", "neu-erzeugt", "123456")
                .unwrap(),
            "z-1"
        );
        h.join().unwrap();
        let (mut v, h) = paar("neu-erzeugt", vec![]);
        let e = v
            .koppeln("corporate-llm", "Corporate LLM", "neu-erzeugt", "000000")
            .unwrap_err();
        assert_eq!(e.code(), code::ABGELEHNT);
        h.join().unwrap();
    }

    /// Liest höchstens ein Byte je Aufruf (Teillieferungen).
    struct Tropfen(UnixStream);
    impl Read for Tropfen {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(1);
            self.0.read(&mut buf[..n])
        }
    }
    impl Write for Tropfen {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }

    #[test]
    fn teillieferungen_byteweise() {
        let (a, b) = UnixStream::pair().unwrap();
        let h = thread::spawn(move || dienst(b, "geheim", vec![Recht::Hoeren]));
        let mut v = Verbindung::neu(Tropfen(a));
        v.anmelden("corporate-llm", "z-1", "geheim").unwrap();
        let text = v
            .erkennen(1, "allgemein", Souveraenitaet::NurLokal, &[0; 64], |_| {})
            .unwrap();
        assert_eq!(text, "64 Bytes");
        drop(v);
        h.join().unwrap();
    }

    fn sende(s: &mut UnixStream, m: &AnApp) {
        let r = Rahmen::steuer(m.strom(), serde_json::to_vec(m).unwrap());
        s.write_all(&r.kodieren().unwrap()).unwrap();
    }

    fn angemeldet() -> (Verbindung<UnixStream>, UnixStream) {
        let (a, mut b) = UnixStream::pair().unwrap();
        sende(
            &mut b,
            &AnApp::Gruss {
                protokoll: crate::PROTOKOLL,
                zufall: zufall(),
            },
        );
        sende(
            &mut b,
            &AnApp::Willkommen {
                protokoll: crate::PROTOKOLL,
                dienst: "t".into(),
                version: "0".into(),
                rechte: vec![Recht::Hoeren],
                engines: Engines {
                    erkennung: "e".into(),
                    stimme: "s".into(),
                    lokal: true,
                },
                beweis: beweis::dienst_erzeugen(
                    &beweis::fingerabdruck("k"),
                    &zufall(),
                    "corporate-llm",
                ),
            },
        );
        let mut v = Verbindung::neu(a);
        v.anmelden("corporate-llm", "z-1", "k").unwrap();
        (v, b)
    }

    #[test]
    fn ergebnis_waehrend_der_aufnahme_wird_geliefert() {
        let (mut v, mut b) = angemeldet();
        // Kein Guthaben, sofort Ergebnis (Höchstdauer erreicht): der Client hängt nicht.
        sende(
            &mut b,
            &AnApp::Ergebnis {
                strom: 1,
                text: "früh".into(),
            },
        );
        let t = v
            .erkennen(1, "allgemein", Souveraenitaet::NurLokal, &[0; 4], |_| {})
            .unwrap();
        assert_eq!(t, "früh");
        // Ein später Fehler des beendeten Stroms ist kein Vertragsbruch.
        sende(
            &mut b,
            &AnApp::Fehler {
                strom: 1,
                code: code::ABGEBROCHEN.into(),
                nachricht: "".into(),
            },
        );
        sende(
            &mut b,
            &AnApp::Status {
                bereit: true,
                engines: Engines {
                    erkennung: "e".into(),
                    stimme: "s".into(),
                    lokal: true,
                },
            },
        );
        assert!(matches!(
            v.empfangen().unwrap(),
            Ereignis::Steuer(AnApp::Fehler { .. })
        ));
        assert!(matches!(
            v.empfangen().unwrap(),
            Ereignis::Steuer(AnApp::Status { .. })
        ));
        // Stromnummer nicht wiederverwendbar.
        assert_eq!(
            v.erkennen(1, "allgemein", Souveraenitaet::NurLokal, &[0; 4], |_| {})
                .unwrap_err()
                .to_string(),
            "Vertrag verletzt: strom_belegt"
        );
    }

    #[test]
    fn fremde_stroeme_und_audio_heraus_sind_vertragsbruch() {
        let (mut v, mut b) = angemeldet();
        sende(
            &mut b,
            &AnApp::Teil {
                strom: 7,
                text: "x".into(),
            },
        );
        assert!(matches!(v.empfangen(), Err(ClientFehler::Vertrag(_))));

        let (mut v, mut b) = angemeldet();
        let r = Rahmen {
            art: Art::AudioHeraus,
            strom: 1,
            nutzlast: vec![1, 2],
        };
        b.write_all(&r.kodieren().unwrap()).unwrap();
        assert!(matches!(v.empfangen(), Err(ClientFehler::Vertrag(_))));
    }

    #[test]
    fn fehler_auf_strom_null_beendet_erkennen() {
        let (mut v, mut b) = angemeldet();
        sende(
            &mut b,
            &AnApp::Fehler {
                strom: 0,
                code: code::SCHLUESSEL_WIDERRUFEN.into(),
                nachricht: "".into(),
            },
        );
        let e = v
            .erkennen(2, "allgemein", Souveraenitaet::NurLokal, &[0; 4], |_| {})
            .unwrap_err();
        assert_eq!(e.code(), code::SCHLUESSEL_WIDERRUFEN);
    }

    #[test]
    fn dienst_ohne_gegenbeweis_wird_abgewiesen() {
        let (a, mut b) = UnixStream::pair().unwrap();
        sende(
            &mut b,
            &AnApp::Gruss {
                protokoll: crate::PROTOKOLL,
                zufall: zufall(),
            },
        );
        // Ein fremder Prozess kennt den Fingerabdruck nicht und rät den Gegenbeweis.
        sende(
            &mut b,
            &AnApp::Willkommen {
                protokoll: crate::PROTOKOLL,
                dienst: "falsch".into(),
                version: "0".into(),
                rechte: vec![Recht::Hoeren],
                engines: Engines {
                    erkennung: "e".into(),
                    stimme: "s".into(),
                    lokal: true,
                },
                beweis: "0".repeat(64),
            },
        );
        let mut v = Verbindung::neu(a);
        let e = v.anmelden("corporate-llm", "z-1", "k").unwrap_err();
        assert_eq!(e.to_string(), "Vertrag verletzt: dienst_beweis");
    }

    #[test]
    fn anmelden_nur_einmal() {
        let (mut v, _b) = angemeldet();
        assert!(matches!(
            v.anmelden("corporate-llm", "z-1", "k"),
            Err(ClientFehler::Vertrag(_))
        ));
        assert!(matches!(
            v.koppeln("corporate-llm", "X", "k", "123456"),
            Err(ClientFehler::Vertrag(_))
        ));
    }

    #[test]
    fn ungueltige_eingaben_gehen_nicht_raus() {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut v = Verbindung::neu(a);
        assert!(matches!(
            v.rueckmelden("../x", vec![]),
            Err(ClientFehler::Vertrag(_))
        ));
        assert!(matches!(
            v.erkennen(1, "a", Souveraenitaet::NurLokal, &[0], |_| {}),
            Err(ClientFehler::Vertrag(_))
        ));
    }
}
