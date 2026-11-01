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
use crate::kopplung::AppSeite;
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

/// 32 Zufallsbytes als Hex (Zufall der App in `hallo`).
fn zufall_hex() -> Result<String, ClientFehler> {
    use ring::rand::SecureRandom;
    let mut b = [0u8; 32];
    ring::rand::SystemRandom::new()
        .fill(&mut b)
        .map_err(|_| ClientFehler::Vertrag("zufall".into()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// Ergebnis einer Kopplung.
#[derive(Clone, PartialEq, Eq)]
pub struct Gekoppelt {
    /// Öffentliche Kennung für künftige `hallo`.
    pub zugang: String,
    /// Zugangsschlüssel (Hex); gehört in den Tresor, nie in Dateien oder Logs.
    pub schluessel: String,
}

/// Ohne Schlüssel, damit er nie über `{:?}` in Logs oder Panikmeldungen landet.
impl std::fmt::Debug for Gekoppelt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gekoppelt")
            .field("zugang", &self.zugang)
            .field("schluessel", &"…")
            .finish()
    }
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
        let app_zufall = zufall_hex()?;
        let fp = beweis::fingerabdruck(schluessel);
        self.senden(&AnDienst::Hallo {
            protokoll: crate::PROTOKOLL,
            client: client.into(),
            zugang: zugang.into(),
            beweis: beweis::erzeugen(&fp, &zufall, client),
            zufall: app_zufall.clone(),
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
                if !beweis::dienst_pruefen(&fp, &zufall, &app_zufall, client, &gegen) {
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

    /// Kopplung mit Schlüsselaustausch ([`crate::kopplung`]). `vergleich` bekommt den Code, den
    /// die App anzeigen soll, sobald er feststeht; danach wartet die Verbindung, bis der Nutzer in
    /// Local Voice AI bestätigt. Liefert Zugang und Schlüssel (der Schlüssel gehört in den Tresor).
    pub fn koppeln(
        &mut self,
        client: &str,
        anzeige: &str,
        vergleich: impl FnOnce(&str),
    ) -> Result<Gekoppelt, ClientFehler> {
        let zufall = self.gruss()?;
        let app = AppSeite::neu().map_err(|_| ClientFehler::Vertrag("kopplung".into()))?;
        self.senden(&AnDienst::Koppeln {
            protokoll: crate::PROTOKOLL,
            client: client.into(),
            anzeige: anzeige.into(),
            zusage: app.zusage(),
        })?;
        let dienst_schluessel = match self.steuer()? {
            AnApp::KopplungAngebot { dienst_schluessel } => dienst_schluessel,
            _ => return Err(ClientFehler::Vertrag("kopplung_angebot".into())),
        };
        self.senden(&AnDienst::KopplungSchluessel {
            app_schluessel: app.oeffentlich(),
        })?;
        let ergebnis = app
            .abschliessen(&dienst_schluessel, client, &zufall)
            .map_err(|_| ClientFehler::Vertrag("kopplung".into()))?;
        vergleich(&ergebnis.vergleich);
        match self.steuer()? {
            AnApp::Gekoppelt { zugang } => Ok(Gekoppelt {
                zugang,
                schluessel: ergebnis.schluessel,
            }),
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
    use crate::kopplung::DienstSeite;
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
        let mut kopplung: Option<(DienstSeite, String, String)> = None;
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
                    client,
                    beweis: b,
                    zufall: app_zufall,
                    ..
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
                                beweis: beweis::dienst_erzeugen(
                                    &fp,
                                    &zufall(),
                                    &app_zufall,
                                    &client,
                                ),
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
                AnDienst::Koppeln { client, zusage, .. } => {
                    let seite = DienstSeite::neu().unwrap();
                    sende(
                        &mut s,
                        AnApp::KopplungAngebot {
                            dienst_schluessel: seite.oeffentlich(),
                        },
                    );
                    kopplung = Some((seite, zusage, client));
                }
                AnDienst::KopplungSchluessel { app_schluessel } => {
                    let (seite, zusage, client) = kopplung.take().unwrap();
                    let e = seite
                        .abschliessen(&app_schluessel, &zusage, &client, &zufall())
                        .unwrap();
                    log.push(format!("vergleich:{}", e.vergleich));
                    log.push(format!("schluessel:{}", e.schluessel));
                    // Testkonvention: Ein Testdienst mit Schlüssel `ablehnen` lehnt ab.
                    if schluessel != "ablehnen" {
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
    fn koppeln_tauscht_schluessel_und_zeigt_denselben_code() {
        let (mut v, h) = paar("annehmen", vec![]);
        let mut gesehen = String::new();
        let g = v
            .koppeln("corporate-llm", "Corporate LLM", |c| gesehen = c.to_owned())
            .unwrap();
        drop(v);
        let log = h.join().unwrap();
        assert_eq!(g.zugang, "z-1");
        assert_eq!(gesehen.len(), 6);
        assert!(log.contains(&format!("vergleich:{gesehen}")));
        assert!(log.contains(&format!("schluessel:{}", g.schluessel)));
        // Zweite Kopplung: neuer Schlüssel.
        let (mut v, h) = paar("annehmen", vec![]);
        let g2 = v.koppeln("corporate-llm", "Corporate LLM", |_| {}).unwrap();
        drop(v);
        h.join().unwrap();
        assert_ne!(g.schluessel, g2.schluessel);

        let (mut v, h) = paar("ablehnen", vec![]);
        let e = v
            .koppeln("corporate-llm", "Corporate LLM", |_| {})
            .unwrap_err();
        assert_eq!(e.code(), code::ABGELEHNT);
        drop(v);
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

    /// Liest das `hallo` der App (Dienstseite) und liefert ihren Zufall.
    fn lies_hallo(b: &mut UnixStream) -> String {
        let mut d = Decoder::new(Rolle::Dienst);
        let mut buf = [0u8; 1024];
        loop {
            if let Some(r) = d.naechster().unwrap() {
                let m: AnDienst = serde_json::from_slice(&r.nutzlast).unwrap();
                let AnDienst::Hallo { zufall, .. } = m else {
                    panic!("hallo erwartet");
                };
                return zufall;
            }
            let n = b.read(&mut buf).unwrap();
            assert!(n > 0);
            let mut rest = &buf[..n];
            while !rest.is_empty() {
                let k = d.einspeisen(rest).unwrap();
                rest = &rest[k..];
                if k == 0 {
                    break;
                }
            }
        }
    }

    fn willkommen(beweis: String) -> AnApp {
        AnApp::Willkommen {
            protokoll: crate::PROTOKOLL,
            dienst: "t".into(),
            version: "0".into(),
            rechte: vec![Recht::Hoeren],
            engines: Engines {
                erkennung: "e".into(),
                stimme: "s".into(),
                lokal: true,
            },
            beweis,
        }
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
        let h = thread::spawn(move || {
            let mut v = Verbindung::neu(a);
            v.anmelden("corporate-llm", "z-1", "k").unwrap();
            v
        });
        let app_zufall = lies_hallo(&mut b);
        sende(
            &mut b,
            &willkommen(beweis::dienst_erzeugen(
                &beweis::fingerabdruck("k"),
                &zufall(),
                &app_zufall,
                "corporate-llm",
            )),
        );
        (h.join().unwrap(), b)
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
        // Wiedergabe: ein Gegenbeweis zu einem fremden App-Zufall gilt nicht.
        let (a, mut b) = UnixStream::pair().unwrap();
        sende(
            &mut b,
            &AnApp::Gruss {
                protokoll: crate::PROTOKOLL,
                zufall: zufall(),
            },
        );
        let alt = beweis::dienst_erzeugen(
            &beweis::fingerabdruck("k"),
            &zufall(),
            &"ee".repeat(32),
            "corporate-llm",
        );
        sende(&mut b, &willkommen(alt));
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
            v.koppeln("corporate-llm", "X", |_| {}),
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
