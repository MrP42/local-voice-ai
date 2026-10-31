//! Eine Verbindung zum Sprachdienst (ADR-0021, Protokoll `sprachdienst-protokoll.md`).
//!
//! Die Schicht darunter (Pipe oder Unix-Socket) liefert einen beliebigen asynchronen Bytestrom;
//! hier läuft das Protokoll: `gruss` → `hallo` (HMAC-Beweis) oder `koppeln` → Ströme für Hören und
//! Sprechen mit Guthaben als Rückstau. Eine Verbindung ist eine Aufgabe, alle Zustände gehören ihr.
//!
//! Festlegungen, die der Vertrag offenlässt oder ausdrücklich verlangt:
//!
//! - **Startguthaben ausdrücklich**: Nach `start` (Erkennen) sendet der Dienst `guthaben` mit
//!   `GUTHABEN_START`. Bis dahin hat der Strom kein Guthaben; Audio davor ist `kein_guthaben`.
//! - **`fehler` auf Strom 0 beendet die Verbindung**, auch bei Vertragsbrüchen (falsche Reihenfolge,
//!   unbekannter Strom, ungültiges Feld). Fehler eines einzelnen Stroms (`recht_fehlt`,
//!   `nicht_lokal`, `engine_fehlt`, `fehlgeschlagen`, `zeit_ueberschritten`, `abgebrochen`) tragen
//!   die Stromnummer und lassen die Verbindung bestehen.
//! - **Rechte und Widerruf** werden bei jedem `start` neu aus dem Speicher gelesen; ein Widerruf
//!   beendet außerdem laufende Verbindungen sofort (`Kontext::verbindungen_beenden`).
//! - **Audio nach Ende eines Stroms** (etwa nach der Höchstdauer, während noch Rahmen unterwegs
//!   sind) wird still verworfen; Audio für nie geöffnete Ströme ist `strom_unbekannt`.
//! - **Audio heraus** wird als vollständige WAV-Häppchen geliefert (siehe `wav`), `fertig` erst,
//!   wenn alle Häppchen den Ausgang verlassen haben.
//!
//! Keine Inhalte im Log: weder Audio noch Text. Das Rohaudio liegt nur im Arbeitsspeicher dieser
//! Aufgabe und wird nach dem Erkennen verworfen; es wird nirgends gespeichert.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rand::rngs::OsRng;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, sleep, timeout, MissedTickBehavior};
use wai_voice_dienst::beweis;
use wai_voice_dienst::fluss::{Ausgang, Einreihen, Guthaben};
use wai_voice_dienst::frame::{Art, Decoder, Rahmen, RahmenFehler, Rolle};
use wai_voice_dienst::grenzen;
use wai_voice_dienst::msg::{
    code, pruefen, AnApp, AnDienst, Operation, Recht, Sitzung, Souveraenitaet, Wortpaar,
};
use wai_voice_dienst::PROTOKOLL;

use super::engine::EngineFehler;
use super::kontext::{Kontext, Meldung};
use super::wav;
use super::zugang::{fingerabdruck_bytes, Fehler as ZugangFehler, Zugang};

/// Die Verbindung ist zu Ende (Fehler gesendet, Gegenstelle weg, Dienst gestoppt).
struct Beendet;
type Res = Result<(), Beendet>;

enum LeseFehler {
    Io,
    Rahmen(RahmenFehler),
}

/// Liest Rahmen aus einem Bytestrom, der in beliebigen Stücken ankommt.
struct Leser<R> {
    r: R,
    dec: Decoder,
    rest: Vec<u8>,
}

impl<R: AsyncRead + Unpin> Leser<R> {
    fn neu(r: R) -> Self {
        Self {
            r,
            dec: Decoder::new(Rolle::Dienst),
            rest: Vec::new(),
        }
    }

    /// Nächster Rahmen; `None` bei sauberem Ende der Gegenstelle. Abbruchsicher: Der Zustand
    /// liegt in `self`, nur das Warten auf Bytes wird unterbrochen.
    async fn naechster(&mut self) -> Result<Option<Rahmen>, LeseFehler> {
        loop {
            if !self.rest.is_empty() {
                let n = self
                    .dec
                    .einspeisen(&self.rest)
                    .map_err(LeseFehler::Rahmen)?;
                self.rest.drain(..n);
            }
            if let Some(r) = self.dec.naechster().map_err(LeseFehler::Rahmen)? {
                return Ok(Some(r));
            }
            if !self.rest.is_empty() {
                continue;
            }
            let mut buf = [0u8; 8192];
            let n = self.r.read(&mut buf).await.map_err(|_| LeseFehler::Io)?;
            if n == 0 {
                return Ok(None);
            }
            self.rest.extend_from_slice(&buf[..n]);
        }
    }
}

enum Typ {
    Erkennen { teil: bool },
    Sprechen,
}

struct Strom {
    gen: u64,
    typ: Typ,
    bereich: String,
    beginn: Instant,
    /// Letzter Rahmen der App für diesen Strom.
    aktiv: Instant,
    pcm: Vec<u8>,
    /// Audiorahmen seit dem letzten gewährten Guthaben.
    abgeholt: u32,
    ende: bool,
    teil_laeuft: bool,
    teil_bytes: usize,
    teil_text: String,
    wartend: VecDeque<Vec<u8>>,
    synth_fertig: bool,
}

enum Ev {
    Teil {
        strom: u32,
        gen: u64,
        text: Result<String, EngineFehler>,
    },
    Ergebnis {
        strom: u32,
        gen: u64,
        text: Result<String, EngineFehler>,
    },
    Sprechen {
        strom: u32,
        gen: u64,
        haeppchen: Result<Vec<Vec<u8>>, EngineFehler>,
    },
}

struct Verb<W> {
    ctx: Arc<Kontext>,
    schreib: W,
    ausgang: Ausgang,
    sitzung: Sitzung,
    zugang: Option<String>,
    zufall: String,
    /// Guthaben für Audio hinein: gewährt der Dienst.
    eingang: Guthaben,
    /// Guthaben für Audio heraus: gewährt die App.
    heraus: Guthaben,
    stroeme: HashMap<u32, Strom>,
    /// Alle Stromnummern, die diese Verbindung je geöffnet hat.
    benutzt: HashSet<u32>,
    tx: mpsc::UnboundedSender<Ev>,
    gen: u64,
    beenden: bool,
}

/// Bedient eine Verbindung bis zum Ende. Gibt den Platz in `Kontext::platz` erst mit dem Ende frei
/// (der Aufrufer hält die Erlaubnis).
pub async fn verbindung<S>(strom: S, ctx: Arc<Kontext>, stopp: watch::Receiver<bool>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (r, w) = tokio::io::split(strom);
    let (tx, rx) = mpsc::unbounded_channel();
    let mut v = Verb {
        ctx,
        schreib: w,
        ausgang: Ausgang::new(),
        sitzung: Sitzung::new(),
        zugang: None,
        zufall: String::new(),
        eingang: Guthaben::default(),
        heraus: Guthaben::default(),
        stroeme: HashMap::new(),
        benutzt: HashSet::new(),
        tx,
        gen: 0,
        beenden: false,
    };
    v.lauf(Leser::neu(r), rx, stopp).await;
    let _ = v.schreib.shutdown().await;
}

fn neuer_zufall() -> String {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn gekuerzt(text: String) -> String {
    let grenze = grenzen::TEXT_MAX * 4;
    if text.chars().count() <= grenze {
        return text;
    }
    text.chars().take(grenze).collect()
}

enum Entscheidung {
    Ja,
    Nein,
    Zeit,
    Aufgelegt,
    Verletzung,
}

impl<W: AsyncWrite + Unpin> Verb<W> {
    async fn lauf<R: AsyncRead + Unpin>(
        &mut self,
        mut leser: Leser<R>,
        mut rx: mpsc::UnboundedReceiver<Ev>,
        mut stopp: watch::Receiver<bool>,
    ) {
        self.zufall = neuer_zufall();
        let gruss = AnApp::Gruss {
            protokoll: PROTOKOLL,
            zufall: self.zufall.clone(),
        };
        if self.senden(0, &gruss).is_err() {
            return;
        }
        let mut widerruf = self.ctx.abonnieren();
        let mut takt = interval(self.ctx.konfig.takt);
        takt.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // Der erste Takt kommt sofort; er soll nicht mitten in den ersten Rahmen fallen.
        takt.tick().await;
        let mut letzte = Instant::now();
        loop {
            if self.ausliefern().await.is_err() || self.beenden {
                return;
            }
            let schritt: Res = tokio::select! {
                r = leser.naechster() => match r {
                    Ok(Some(rahmen)) => {
                        letzte = Instant::now();
                        self.rahmen(rahmen, &mut leser, &mut stopp).await
                    }
                    Ok(None) | Err(LeseFehler::Io) => return,
                    Err(LeseFehler::Rahmen(f)) => {
                        log::info!("sprachdienst: Rahmenfehler: {f}");
                        self.fatal(code::UNGUELTIG, "Der Rahmen verletzt den Vertrag.")
                    }
                },
                Some(ev) = rx.recv() => self.ereignis(ev),
                _ = takt.tick() => self.takt(letzte),
                w = widerruf.recv() => match w {
                    Ok(id) if self.zugang.as_deref() == Some(id.as_str()) => {
                        self.fatal(code::SCHLUESSEL_WIDERRUFEN, "Der Zugang wurde widerrufen.")
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => self.zugang_pruefen(),
                    _ => Ok(()),
                },
                _ = stopp.changed() => return,
            };
            if schritt.is_err() {
                let _ = self.fluten().await;
                return;
            }
        }
    }

    // --- Ausgang -------------------------------------------------------------------------

    /// Steuernachricht einreihen. Zu große oder überzählige Rahmen beenden die Verbindung.
    fn senden(&mut self, strom: u32, m: &AnApp) -> Res {
        let Ok(json) = serde_json::to_vec(m) else {
            self.beenden = true;
            return Err(Beendet);
        };
        if json.len() > grenzen::STEUER_MAX {
            self.beenden = true;
            return Err(Beendet);
        }
        match self.ausgang.steuer(Rahmen::steuer(strom, json)) {
            Einreihen::Ok => Ok(()),
            Einreihen::Voll => {
                self.beenden = true;
                Err(Beendet)
            }
        }
    }

    /// Wartendes schreiben und Audio heraus nachschieben, bis nichts mehr geht: erst die
    /// Häppchen, danach (wenn sie den Ausgang verlassen haben) `fertig`.
    async fn ausliefern(&mut self) -> Res {
        for _ in 0..3 {
            self.fluten().await?;
            if self.beenden {
                return Err(Beendet);
            }
            if self.pumpen().is_err() {
                let _ = self.fluten().await;
                return Err(Beendet);
            }
        }
        self.fluten().await
    }

    /// Alles Wartende schreiben. Eine Gegenstelle, die nicht liest, beendet die Verbindung nach
    /// `Konfig::schreiben`.
    async fn fluten(&mut self) -> Res {
        while let Some(r) = self.ausgang.naechster() {
            let Ok(bytes) = r.kodieren() else { continue };
            let frist = self.ctx.konfig.schreiben;
            if !matches!(
                timeout(frist, self.schreib.write_all(&bytes)).await,
                Ok(Ok(()))
            ) {
                return Err(Beendet);
            }
        }
        if !matches!(
            timeout(self.ctx.konfig.schreiben, self.schreib.flush()).await,
            Ok(Ok(()))
        ) {
            return Err(Beendet);
        }
        Ok(())
    }

    /// Fehler der ganzen Verbindung: gesendet auf Strom 0, danach ist Schluss.
    fn fatal(&mut self, code: &'static str, nachricht: &str) -> Res {
        log::info!("sprachdienst: Verbindung beendet code={code}");
        let _ = self.senden(
            0,
            &AnApp::Fehler {
                strom: 0,
                code: code.to_string(),
                nachricht: nachricht.to_string(),
            },
        );
        self.beenden = true;
        Err(Beendet)
    }

    /// Fehler eines Stroms: Strom schließen, wartendes Audio dazu verwerfen, Fehler melden.
    fn strom_fehler(&mut self, strom: u32, code: &'static str, nachricht: &str) -> Res {
        self.strom_schliessen(strom);
        let m = AnApp::Fehler {
            strom,
            code: code.to_string(),
            nachricht: nachricht.to_string(),
        };
        let Ok(json) = serde_json::to_vec(&m) else {
            return Err(Beendet);
        };
        self.ausgang.abbrechen(strom, Rahmen::steuer(strom, json));
        Ok(())
    }

    fn strom_schliessen(&mut self, strom: u32) {
        self.stroeme.remove(&strom);
        self.sitzung.schliessen(strom);
        self.eingang.schliessen(strom);
        self.heraus.schliessen(strom);
    }

    // --- Eingang -------------------------------------------------------------------------

    async fn rahmen<R: AsyncRead + Unpin>(
        &mut self,
        r: Rahmen,
        leser: &mut Leser<R>,
        stopp: &mut watch::Receiver<bool>,
    ) -> Res {
        match r.art {
            Art::Steuer => self.steuer(r.strom, &r.nutzlast, leser, stopp).await,
            Art::AudioHinein => self.audio(r.strom, r.nutzlast),
            Art::AudioHeraus => self.fatal(code::UNGUELTIG, "Audio in falscher Richtung."),
        }
    }

    async fn steuer<R: AsyncRead + Unpin>(
        &mut self,
        kopf_strom: u32,
        nutzlast: &[u8],
        leser: &mut Leser<R>,
        stopp: &mut watch::Receiver<bool>,
    ) -> Res {
        let Ok(m) = serde_json::from_slice::<AnDienst>(nutzlast) else {
            return self.fatal(code::UNGUELTIG, "Die Nachricht ist nicht lesbar.");
        };
        if let Err(feld) = pruefen(&m) {
            let c = if feld == "protokoll" {
                code::VERSION_INKOMPATIBEL
            } else {
                code::UNGUELTIG
            };
            return self.fatal(c, &format!("Ungültiges Feld: {feld}."));
        }
        if let Err(c) = self.sitzung.annehmen(kopf_strom, &m) {
            return self.fatal(c, "Die Nachricht passt nicht zum Stand der Verbindung.");
        }
        match m {
            AnDienst::Hallo {
                client,
                zugang,
                beweis,
                ..
            } => self.hallo(&client, &zugang, &beweis),
            AnDienst::Koppeln {
                client,
                anzeige,
                fingerabdruck,
                code: code_text,
                ..
            } => {
                self.koppeln(&client, &anzeige, &fingerabdruck, &code_text, leser, stopp)
                    .await
            }
            AnDienst::Start {
                strom,
                op,
                bereich,
                souveraenitaet,
            } => self.start(strom, op, bereich, souveraenitaet),
            AnDienst::Ende { strom } => self.ende(strom),
            AnDienst::Abbrechen { strom } => self.abbrechen(strom),
            AnDienst::Guthaben { strom, rahmen } => self.guthaben(strom, rahmen),
            AnDienst::Rueckmeldung { bereich, paare } => self.rueckmeldung(&bereich, &paare),
            AnDienst::Status {} => {
                let m = AnApp::Status {
                    bereit: self.ctx.engine.bereit(),
                    engines: self.ctx.engine.engines(),
                };
                self.senden(0, &m)
            }
        }
    }

    // --- Handschlag und Kopplung ------------------------------------------------------------------

    fn hallo(&mut self, client: &str, zugang: &str, beweis_hex: &str) -> Res {
        let z = self.ctx.zugaenge.get(zugang);
        // Auch für einen unbekannten Zugang wird gerechnet: gleiche Zeit, gleiche Antwort.
        let fp = z
            .as_ref()
            .and_then(|z| fingerabdruck_bytes(&z.fingerabdruck))
            .unwrap_or([0u8; 32]);
        let gueltig = beweis::pruefen(&fp, &self.zufall, client, beweis_hex);
        let Some(z) = z.filter(|z| gueltig && z.client == client) else {
            return self.fatal(
                code::SCHLUESSEL_UNGUELTIG,
                "Zugang oder Schlüssel ungültig.",
            );
        };
        if !z.aktiv() {
            return self.fatal(code::SCHLUESSEL_WIDERRUFEN, "Der Zugang wurde widerrufen.");
        }
        self.sitzung.anmelden();
        self.zugang = Some(z.zugang.clone());
        self.ctx.zugaenge.beruehren(&z.zugang, (self.ctx.uhr)());
        log::info!("sprachdienst: angemeldet zugang={}", z.zugang);
        let willkommen = AnApp::Willkommen {
            protokoll: PROTOKOLL,
            dienst: self.ctx.konfig.dienst.clone(),
            version: self.ctx.konfig.version.clone(),
            rechte: z.rechte.clone(),
            engines: self.ctx.engine.engines(),
            // Gegenbeweis erst nach erfolgreich geprüftem `hallo`: wer den Fingerabdruck nicht
            // kennt (ein Prozess, der den Endpunkt vorbelegt hat), kann ihn nicht erzeugen.
            beweis: beweis::dienst_erzeugen(&fp, &self.zufall, client),
        };
        self.senden(0, &willkommen)
    }

    async fn koppeln<R: AsyncRead + Unpin>(
        &mut self,
        client: &str,
        anzeige: &str,
        fingerabdruck: &str,
        code_text: &str,
        leser: &mut Leser<R>,
        stopp: &mut watch::Receiver<bool>,
    ) -> Res {
        let jetzt = (self.ctx.uhr)();
        let frist = self.ctx.konfig.bestaetigung;
        let bereits = self.ctx.zugaenge.bereits_gekoppelt(fingerabdruck);
        let mut warten = match self.ctx.kopplungen.anfragen(
            client,
            anzeige,
            code_text,
            bereits,
            jetzt,
            frist.as_millis() as i64,
        ) {
            Ok(w) => w,
            Err(c) => {
                log::info!("sprachdienst: Kopplung abgewiesen code={c}");
                return self.fatal(c, "Die Kopplung ist gerade nicht möglich.");
            }
        };
        log::info!("sprachdienst: Kopplung angefragt client={client}");
        self.ctx.melden(Meldung::KopplungOffen);

        let entscheidung = tokio::select! {
            r = &mut warten.antwort => match r {
                Ok(true) => Entscheidung::Ja,
                _ => Entscheidung::Nein,
            },
            _ = sleep(frist) => Entscheidung::Zeit,
            r = leser.naechster() => match r {
                Ok(Some(_)) => Entscheidung::Verletzung,
                _ => Entscheidung::Aufgelegt,
            },
            _ = stopp.changed() => Entscheidung::Aufgelegt,
        };
        self.ctx.kopplungen.aufgeben(&warten.id);
        // Die Liste der offenen Anfragen hat sich geändert (der Dialog darf verschwinden).
        self.ctx.melden(Meldung::KopplungOffen);

        match entscheidung {
            Entscheidung::Ja => {
                let neu =
                    self.ctx
                        .zugaenge
                        .hinzufuegen(client, anzeige, fingerabdruck, (self.ctx.uhr)());
                match neu {
                    Ok(z) => {
                        log::info!("sprachdienst: gekoppelt zugang={}", z.zugang);
                        self.ctx.melden(Meldung::ZugaengeGeaendert);
                        let r = self.senden(0, &AnApp::Gekoppelt { zugang: z.zugang });
                        // Danach endet die Verbindung.
                        self.beenden = true;
                        r
                    }
                    Err(ZugangFehler::BereitsGekoppelt) => {
                        self.fatal(code::BEREITS_GEKOPPELT, "Diese App ist schon gekoppelt.")
                    }
                    Err(ZugangFehler::ZuViele) => {
                        self.fatal(code::BESCHAEFTIGT, "Es sind schon zu viele Apps gekoppelt.")
                    }
                    Err(_) => self.fatal(
                        code::FEHLGESCHLAGEN,
                        "Die Kopplung ließ sich nicht speichern.",
                    ),
                }
            }
            Entscheidung::Nein => self.fatal(code::ABGELEHNT, "Die Kopplung wurde abgelehnt."),
            Entscheidung::Zeit => self.fatal(
                code::ZEIT_UEBERSCHRITTEN,
                "Die Kopplung wurde nicht rechtzeitig bestätigt.",
            ),
            Entscheidung::Verletzung => self.fatal(
                code::UNGUELTIG,
                "Während der Kopplung sind keine Nachrichten erlaubt.",
            ),
            Entscheidung::Aufgelegt => Err(Beendet),
        }
    }

    /// Der Zugang der Verbindung, wenn er noch gilt.
    fn aktueller_zugang(&self) -> Option<Zugang> {
        let id = self.zugang.as_deref()?;
        self.ctx.zugaenge.get(id).filter(Zugang::aktiv)
    }

    fn zugang_pruefen(&mut self) -> Res {
        if self.zugang.is_some() && self.aktueller_zugang().is_none() {
            return self.fatal(code::SCHLUESSEL_WIDERRUFEN, "Der Zugang wurde widerrufen.");
        }
        Ok(())
    }

    // --- Ströme ------------------------------------------------------------------------------------

    fn neuer_strom(&mut self, typ: Typ, bereich: String) -> Strom {
        self.gen += 1;
        let jetzt = Instant::now();
        Strom {
            gen: self.gen,
            typ,
            bereich,
            beginn: jetzt,
            aktiv: jetzt,
            pcm: Vec::new(),
            abgeholt: 0,
            ende: false,
            teil_laeuft: false,
            teil_bytes: 0,
            teil_text: String::new(),
            wartend: VecDeque::new(),
            synth_fertig: false,
        }
    }

    fn start(&mut self, strom: u32, op: Operation, bereich: String, souv: Souveraenitaet) -> Res {
        // Widerruf und Rechte gelten ab dem Moment des Starts, nicht ab dem Handschlag.
        let Some(z) = self.aktueller_zugang() else {
            return self.fatal(code::SCHLUESSEL_WIDERRUFEN, "Der Zugang wurde widerrufen.");
        };
        self.benutzt.insert(strom);
        if !z.hat_recht(op.recht()) {
            return self.strom_fehler(
                strom,
                code::RECHT_FEHLT,
                "Dieses Recht ist in Local Voice AI nicht freigegeben.",
            );
        }
        if souv == Souveraenitaet::NurLokal && !self.ctx.engine.engines().lokal {
            return self.strom_fehler(
                strom,
                code::NICHT_LOKAL,
                "Die Engines laufen nicht ausschließlich auf diesem Rechner.",
            );
        }
        if !self.ctx.engine.bereit() {
            return self.strom_fehler(strom, code::ENGINE_FEHLT, "Es ist keine Engine bereit.");
        }
        log::info!(
            "sprachdienst: start zugang={} strom={strom} op={}",
            z.zugang,
            match &op {
                Operation::Erkennen => "erkennen",
                Operation::ErkennenStrom => "erkennen_strom",
                Operation::Sprechen { .. } => "sprechen",
            }
        );
        let teil = matches!(op, Operation::ErkennenStrom);
        match op {
            Operation::Erkennen | Operation::ErkennenStrom => {
                let s = self.neuer_strom(Typ::Erkennen { teil }, bereich);
                self.stroeme.insert(strom, s);
                // Ausdrücklich: Der Strom beginnt ohne Guthaben, das Startguthaben kommt als Nachricht.
                self.eingang.oeffnen(strom, 0);
                self.eingang.gewaehren(strom, grenzen::GUTHABEN_START);
                self.senden(
                    strom,
                    &AnApp::Guthaben {
                        strom,
                        rahmen: grenzen::GUTHABEN_START,
                    },
                )
            }
            Operation::Sprechen { text, stimme } => {
                let s = self.neuer_strom(Typ::Sprechen, bereich);
                let gen = s.gen;
                self.stroeme.insert(strom, s);
                self.heraus.oeffnen(strom, 0);
                let engine = self.ctx.engine.clone();
                let tx = self.tx.clone();
                tokio::task::spawn_blocking(move || {
                    let haeppchen = engine.sprechen(&text, stimme.as_deref()).and_then(|wavs| {
                        let mut alle = Vec::new();
                        for w in &wavs {
                            let teile = wav::haeppchen(w).map_err(|_| {
                                EngineFehler::fehlgeschlagen(
                                    "Die Stimme lieferte kein gültiges Audio.",
                                )
                            })?;
                            alle.extend(teile);
                        }
                        Ok(alle)
                    });
                    let _ = tx.send(Ev::Sprechen {
                        strom,
                        gen,
                        haeppchen,
                    });
                });
                Ok(())
            }
        }
    }

    fn audio(&mut self, strom: u32, pcm: Vec<u8>) -> Res {
        if !self.sitzung.audio_erlaubt(strom) {
            // Späte Rahmen eines schon beendeten Stroms (Höchstdauer, Fehler) sind kein Vertragsbruch.
            if self.benutzt.contains(&strom) {
                return Ok(());
            }
            return self.fatal(
                code::STROM_UNBEKANNT,
                "Audio für einen Strom, der nie geöffnet wurde.",
            );
        }
        let Some(s) = self.stroeme.get_mut(&strom) else {
            return Ok(());
        };
        if !matches!(s.typ, Typ::Erkennen { .. }) || s.ende {
            return self.strom_fehler(strom, code::UNGUELTIG, "Hier ist kein Audio mehr erwartet.");
        }
        if !self.eingang.verbrauchen(strom) {
            return self.fatal(code::KEIN_GUTHABEN, "Audio ohne Guthaben.");
        }
        s.aktiv = Instant::now();
        s.pcm.extend_from_slice(&pcm);
        s.abgeholt += 1;
        let voll = s.pcm.len() >= self.ctx.konfig.aeusserung_max_s as usize * grenzen::AUDIO_MAX;
        if voll {
            // Höchstdauer der Äußerung: erkennen, was da ist; spätere Rahmen werden verworfen.
            return self.erkennung_abschliessen(strom);
        }
        Ok(())
    }

    fn ende(&mut self, strom: u32) -> Res {
        let Some(s) = self.stroeme.get(&strom) else {
            return self.fatal(code::STROM_UNBEKANNT, "Dieser Strom ist nicht offen.");
        };
        if !matches!(s.typ, Typ::Erkennen { .. }) {
            return self.strom_fehler(
                strom,
                code::UNGUELTIG,
                "Dieser Strom nimmt kein Ende der Eingabe an.",
            );
        }
        if s.ende {
            return Ok(());
        }
        self.erkennung_abschliessen(strom)
    }

    /// Die Eingabe ist zu Ende: Das Gesammelte wird erkannt, das Rohaudio danach verworfen.
    fn erkennung_abschliessen(&mut self, strom: u32) -> Res {
        let Some(s) = self.stroeme.get_mut(&strom) else {
            return Ok(());
        };
        s.ende = true;
        let pcm = std::mem::take(&mut s.pcm);
        let (gen, bereich) = (s.gen, s.bereich.clone());
        let engine = self.ctx.engine.clone();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let text = engine.erkennen(&pcm, &bereich);
            drop(pcm);
            let _ = tx.send(Ev::Ergebnis { strom, gen, text });
        });
        Ok(())
    }

    fn abbrechen(&mut self, strom: u32) -> Res {
        if !self.stroeme.contains_key(&strom) {
            // Ein Abbruch darf vor `start` eintreffen; zu unbekannten Strömen gibt es keine Antwort.
            return Ok(());
        }
        log::info!("sprachdienst: abgebrochen strom={strom}");
        self.strom_fehler(strom, code::ABGEBROCHEN, "Der Strom wurde abgebrochen.")
    }

    fn guthaben(&mut self, strom: u32, rahmen: u32) -> Res {
        let spricht = self
            .stroeme
            .get(&strom)
            .map(|s| matches!(s.typ, Typ::Sprechen));
        match spricht {
            Some(true) => {
                if let Some(s) = self.stroeme.get_mut(&strom) {
                    s.aktiv = Instant::now();
                }
                self.heraus.gewaehren(strom, rahmen);
                self.pumpen()
            }
            Some(false) => self.strom_fehler(
                strom,
                code::UNGUELTIG,
                "Dieser Strom nimmt kein Guthaben an.",
            ),
            None => self.fatal(code::STROM_UNBEKANNT, "Dieser Strom ist nicht offen."),
        }
    }

    fn rueckmeldung(&mut self, bereich: &str, paare: &[Wortpaar]) -> Res {
        let darf = self
            .aktueller_zugang()
            .is_some_and(|z| z.hat_recht(Recht::Lernen));
        if !darf {
            return self.fatal(
                code::RECHT_FEHLT,
                "Das Recht zum Lernen ist nicht freigegeben.",
            );
        }
        match self.ctx.engine.rueckmeldung(bereich, paare.len()) {
            Ok(()) => Ok(()),
            Err(f) => self.fatal(f.code, &f.nachricht),
        }
    }

    // --- Ergebnisse der Engines --------------------------------------------------------------------

    fn ereignis(&mut self, ev: Ev) -> Res {
        match ev {
            Ev::Teil { strom, gen, text } => {
                let Some(s) = self.stroeme.get_mut(&strom).filter(|s| s.gen == gen) else {
                    return Ok(());
                };
                s.teil_laeuft = false;
                let Ok(text) = text else { return Ok(()) };
                if s.ende || text.is_empty() || text == s.teil_text {
                    return Ok(());
                }
                s.teil_text = text.clone();
                self.senden(
                    strom,
                    &AnApp::Teil {
                        strom,
                        text: gekuerzt(text),
                    },
                )
            }
            Ev::Ergebnis { strom, gen, text } => {
                let Some(s) = self.stroeme.get(&strom).filter(|s| s.gen == gen) else {
                    return Ok(());
                };
                let dauer_ms = s.beginn.elapsed().as_millis();
                self.strom_schliessen(strom);
                match text {
                    Ok(text) => {
                        log::info!("sprachdienst: erkannt strom={strom} dauer_ms={dauer_ms}");
                        self.senden(
                            strom,
                            &AnApp::Ergebnis {
                                strom,
                                text: gekuerzt(text),
                            },
                        )
                    }
                    Err(f) => {
                        log::info!(
                            "sprachdienst: Erkennung ohne Ergebnis strom={strom} code={}",
                            f.code
                        );
                        self.senden(
                            strom,
                            &AnApp::Fehler {
                                strom,
                                code: f.code.to_string(),
                                nachricht: f.nachricht,
                            },
                        )
                    }
                }
            }
            Ev::Sprechen {
                strom,
                gen,
                haeppchen,
            } => {
                let Some(s) = self.stroeme.get_mut(&strom).filter(|s| s.gen == gen) else {
                    return Ok(());
                };
                match haeppchen {
                    Ok(h) => {
                        s.wartend = h.into();
                        s.synth_fertig = true;
                        s.aktiv = Instant::now();
                        self.pumpen()
                    }
                    Err(f) => {
                        log::info!(
                            "sprachdienst: Sprechen ohne Ergebnis strom={strom} code={}",
                            f.code
                        );
                        self.strom_fehler(strom, f.code, &f.nachricht)
                    }
                }
            }
        }
    }

    /// Audio heraus: so viele Häppchen einreihen, wie Guthaben und Puffer erlauben; `fertig`, wenn
    /// alles den Ausgang verlassen hat (Steuerrahmen überholen Audio, `fertig` darf nicht vorher gehen).
    fn pumpen(&mut self) -> Res {
        let nummern: Vec<u32> = self
            .stroeme
            .iter()
            .filter(|(_, s)| matches!(s.typ, Typ::Sprechen) && s.synth_fertig)
            .map(|(n, _)| *n)
            .collect();
        for strom in nummern {
            loop {
                let Some(s) = self.stroeme.get_mut(&strom) else {
                    break;
                };
                let Some(laenge) = s.wartend.front().map(Vec::len) else {
                    break;
                };
                if self.ausgang.audio_bytes() + laenge > grenzen::AUDIO_PUFFER_MAX
                    || !self.heraus.verbrauchen(strom)
                {
                    break;
                }
                let Some(h) = s.wartend.pop_front() else {
                    break;
                };
                let r = Rahmen {
                    art: Art::AudioHeraus,
                    strom,
                    nutzlast: h,
                };
                if self.ausgang.audio(r) == Einreihen::Voll {
                    self.beenden = true;
                    return Err(Beendet);
                }
            }
            let fertig = self
                .stroeme
                .get(&strom)
                .is_some_and(|s| s.wartend.is_empty())
                && self.ausgang.audio_bytes() == 0;
            if fertig {
                let dauer_ms = self
                    .stroeme
                    .get(&strom)
                    .map_or(0, |s| s.beginn.elapsed().as_millis());
                self.strom_schliessen(strom);
                log::info!("sprachdienst: gesprochen strom={strom} dauer_ms={dauer_ms}");
                self.senden(strom, &AnApp::Fertig { strom })?;
            }
        }
        Ok(())
    }

    // --- Takt: Fristen, Teilergebnisse, Ausgabe -----------------------------------------------------

    fn takt(&mut self, letzte: Instant) -> Res {
        let k = self.ctx.konfig.clone();
        if !self.sitzung.angemeldet() && letzte.elapsed() > k.anfang {
            return self.fatal(
                code::ZEIT_UEBERSCHRITTEN,
                "Kein Handschlag innerhalb der Frist.",
            );
        }
        if self.sitzung.angemeldet()
            && self.stroeme.is_empty()
            && letzte.elapsed() > k.leerlauf_verbindung
        {
            self.beenden = true;
            return Err(Beendet);
        }
        let hoechstdauer = Duration::from_secs(u64::from(k.aeusserung_max_s));
        let mut zu_spaet = Vec::new();
        let mut abschliessen = Vec::new();
        let mut teil = Vec::new();
        let mut nachschub = Vec::new();
        for (n, s) in &self.stroeme {
            match s.typ {
                Typ::Erkennen { teil: mit_teil } => {
                    if s.ende {
                        continue;
                    }
                    if s.abgeholt > 0 {
                        nachschub.push((*n, s.abgeholt));
                    }
                    if s.beginn.elapsed() > hoechstdauer {
                        abschliessen.push(*n);
                    } else if s.aktiv.elapsed() > k.leerlauf_strom {
                        zu_spaet.push(*n);
                    } else if mit_teil
                        && !s.teil_laeuft
                        && s.pcm.len() >= s.teil_bytes + k.teil_abstand
                    {
                        teil.push(*n);
                    }
                }
                Typ::Sprechen => {
                    if s.synth_fertig
                        && !s.wartend.is_empty()
                        && s.aktiv.elapsed() > k.leerlauf_strom
                    {
                        zu_spaet.push(*n);
                    }
                }
            }
        }
        // Das Guthaben kommt im Takt zurück, nicht je Rahmen: Wer schneller sendet, als er
        // Guthaben hat, bekommt `kein_guthaben` (der Rückstau hat Zähne).
        for (n, anzahl) in nachschub {
            if let Some(s) = self.stroeme.get_mut(&n) {
                s.abgeholt = 0;
            }
            self.eingang.gewaehren(n, anzahl);
            self.senden(
                n,
                &AnApp::Guthaben {
                    strom: n,
                    rahmen: anzahl,
                },
            )?;
        }
        for n in zu_spaet {
            self.strom_fehler(n, code::ZEIT_UEBERSCHRITTEN, "Zu lange ohne Audio.")?;
        }
        for n in abschliessen {
            self.erkennung_abschliessen(n)?;
        }
        for n in teil {
            self.teil_starten(n);
        }
        self.pumpen()
    }

    /// Zwischenergebnis des Diktats: erkennt eine Kopie des bisherigen Audios.
    fn teil_starten(&mut self, strom: u32) {
        let Some(s) = self.stroeme.get_mut(&strom) else {
            return;
        };
        s.teil_laeuft = true;
        s.teil_bytes = s.pcm.len();
        let (pcm, gen, bereich) = (s.pcm.clone(), s.gen, s.bereich.clone());
        let engine = self.ctx.engine.clone();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let text = engine.erkennen(&pcm, &bereich);
            drop(pcm);
            let _ = tx.send(Ev::Teil { strom, gen, text });
        });
    }
}
