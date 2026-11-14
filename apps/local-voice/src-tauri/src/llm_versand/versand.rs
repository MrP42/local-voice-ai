//! [`versenden`]: die gekapselte Versandoperation (lva#604, Teilpaket 3).
//!
//! Prüfen und Senden sind **eine** Operation (Vorbild `mcp/tools.rs:310–319`). Jeder
//! Transportversuch durchläuft in dieser Reihenfolge:
//!
//! 1. [`freigabe_ausstellen`](super::pruefer::freigabe_ausstellen): R1, Quellenprüfung, R2.
//!    `Gesperrt` hat **Vorrang** vor allem Folgenden (auch vor Compliance und Budget).
//! 2. `compliance::check_call` und `usage::check_budget`, bei **jedem** Versuch, auch bei
//!    lokalen Zielen und `Quellen::keine()` (die Besprechungsprüfung entfällt dort, diese beiden
//!    nicht). Fehler sind [`VersandFehler::Compliance`] und [`VersandFehler::Budget`] mit
//!    app-eigener Meldung.
//! 3. **Genau ein** Transportversuch an das gebundene [`Ziel`]; die Freigabe wird dort
//!    eingelöst, die Route nicht neu aufgelöst.
//! 4. [`abschliessen`](super::pruefer::abschliessen): R3, **auch nach einem Transportfehler**,
//!    vor Rückgabe, vor Ausgabe und vor jedem Fallback. R3≠R2 ist `Gesperrt`, terminal; das
//!    Transportergebnis (Antwort wie Fehler) wird verworfen.
//! 5. Erst danach geht gepufferte Ausgabe an die Rückrufe.
//!
//! # Fallbacks
//!
//! Sie liegen **außerhalb** des einen Transportversuchs und sind jeweils ein **neuer** Versuch
//! mit eigener Prüfung (Schritte 1 bis 4): der Strom ohne Strom (abgelehnter Strom oder Abriss
//! vor dem ersten Token) und der Neuversuch einer zu alten Codex-CLI nach dem Update. Weil R3
//! vor der Entscheidung über den Fallback läuft, gibt es nach `Gesperrt` keinen zweiten
//! Request. Die JSON-Reparatur bleibt beim Aufrufer (lva#601); [`mit_wiederholung`] ist ihr
//! Rahmen: jeder Aufruf darin ist ein vollständiges `versenden` mit eigener Prüfung.
//!
//! # Streamingpuffer
//!
//! Bei einem Cloud-Ziel mit `Quellen` ungleich `keine()` gehen **alle** Inhaltsausgaben eines
//! Versuchs (Textstücke, Denktexte, Codex-Denkzeilen) erst nach bestandenem Abschlussvergleich
//! an die Rückrufe, in der Reihenfolge ihres Eintreffens. Bei `Gesperrt` kommt nichts an.
//! Ein Versuch, der nach bestandenem R3 mit einem Fehler endet, gibt seine bis dahin
//! eingetroffene Ausgabe frei: so verhielt sich der Strom bisher (nach dem ersten Token ist ein
//! Abbruch ein Fehler, der Text war schon zu sehen). Lokale Ziele und `keine()` streamen
//! unverändert Stück für Stück. Der Denktextpuffer ist auf [`PUFFER_DENKEN_MAX`] Byte begrenzt;
//! was darüber hinausgeht, wird gezählt, verworfen und einmal gemeldet (nie verschluckt, nie
//! unbegrenzt gehalten). Textstücke begrenzt der Transport selbst (`STREAM_MAX_CHARS`).
//!
//! # Fehlerfälle
//!
//! - **Nebenläufigkeit:** der Puffer eines Versuchs gehört genau diesem Versuch (kein
//!   gemeinsamer Zustand zwischen Aufrufen); die Sperre darin wird nur für ein `push`
//!   gehalten. Widerrufe zwischen den Lesungen sind R1/R2/R3 (siehe [`pruefer`](super::pruefer)).
//! - **Abbruch mitten im Vorgang:** lässt der Aufrufer das Future fallen (Nutzerabbruch), schließt
//!   die Verbindung bzw. endet der CLI-Prozess; der Puffer geht mit dem Future unter, **nichts**
//!   wird mehr ausgegeben. Gebucht wird dann wie bisher nichts (kein Ergebnis).
//! - **Voller Speicher:** begrenzte Puffer (siehe oben); eine vergiftete Puffersperre verwirft
//!   die Ausgabe (sicherer Weg).
//! - **Fehlendes Gerät:** fehlt die CLI, ist das `Cli { code: "cli_missing" }`; ein Ziel ohne
//!   Adresse ist `Transport`.
//! - **Absturz eines Kindprozesses:** der Transport meldet ihn als `Cli`-Fehler ohne Text der
//!   CLI; R3 und die Buchung laufen trotzdem.
//!
//! # Buchung
//!
//! Jeder Aufruf mit mindestens einem tatsächlich ausgeführten Versuch wird **einmal** gebucht
//! (Zweck, Verbrauch aller Versuche, Dauer, Ergebnis), auch wenn das Ergebnis verworfen wurde
//! (`managers/usage.rs:683`) oder scheiterte. Eine Verweigerung vor dem Transport (Gesperrt,
//! Compliance, Budget des ersten Versuchs) bleibt ungebucht: es wurde nichts verbraucht.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::anfrage::{Anfrage, Antwort, ChatInhalt, Inhalt};
use super::fehler::{grund, VersandFehler};
use super::pruefer::{
    abschliessen, besprechung_zu_pruefen, freigabe_ausstellen, freigabe_uebergang, FreigabeQuelle,
    Geprueft, Versandfreigabe,
};
use super::quellen::Quellen;
#[cfg(test)]
use super::testhaken;
use super::transport::{cli_chat, http, Ausgabe, Fehlschlag, Kontext, Versuch};
use super::ziel::Ziel;
use crate::llm_client::{
    dialog_line, ChatReply, DenkSenke, StreamLimits, StreamMessage, CLI_MAX_TOKENS,
    CLI_REQUIRE_USAGE,
};
use crate::managers::llm::cli::Cli;
use crate::managers::llm::cli_update;
use crate::managers::usage::{self, Purpose, TokenUsage};
use crate::settings::PostProcessProvider;

/// Höchstens so viele Byte Denktext hält ein Versuch zurück, bis der Abschlussvergleich
/// bestanden ist. Mehr wird gezählt und verworfen.
pub(super) const PUFFER_DENKEN_MAX: usize = 4 * 1024 * 1024;

/// Wie ein Versuch geprüft wird.
#[derive(Clone, Copy)]
pub(super) enum Pruefung<'a> {
    /// Der volle Vertrag: R1, Quellenprüfung, R2 vor und R3 nach dem Transport.
    Voll(&'a dyn FreigabeQuelle),
    /// **Übergang bis lva#601** ([`uebergang`](super::uebergang)): ohne Besprechungsprüfung,
    /// wie vor diesem Modul. `versenden` benutzt das nie.
    Uebergang,
}

impl Pruefung<'_> {
    /// Müssen die Inhaltsausgaben bis zum bestandenen Abschlussvergleich zurückgehalten werden?
    fn puffert(&self, ziel: &Ziel, quellen: &Quellen) -> bool {
        matches!(self, Self::Voll(_)) && besprechung_zu_pruefen(ziel, quellen)
    }
}

/// Der Ablauf eines Aufrufs über alle seine Versuche: Prüfung, Verbrauch, Buchung.
pub(super) struct Lauf<'a> {
    pruefung: Pruefung<'a>,
    pub(super) ziel: &'a Ziel,
    pub(super) quellen: &'a Quellen,
    anbieter: &'a PostProcessProvider,
    zweck: Purpose,
    start: Instant,
    tokens: TokenUsage,
    ausgefuehrt: bool,
}

impl<'a> Lauf<'a> {
    pub(super) fn neu(
        pruefung: Pruefung<'a>,
        ziel: &'a Ziel,
        quellen: &'a Quellen,
        anbieter: &'a PostProcessProvider,
        zweck: Purpose,
    ) -> Self {
        Self {
            pruefung,
            ziel,
            quellen,
            anbieter,
            zweck,
            start: Instant::now(),
            tokens: TokenUsage::default(),
            ausgefuehrt: false,
        }
    }

    /// Ein vollständiger Versuch: Prüfung, Compliance und Budget, **ein** Transport (`senden`
    /// bekommt die besitzende Freigabe), Abschlussvergleich.
    ///
    /// - Äußeres `Err`: der Versuch ging nicht hinaus (`Gesperrt`, Compliance, Budget) oder sein
    ///   Ergebnis wurde verworfen (R3: `Gesperrt`). Terminal ist nur `Gesperrt`.
    /// - Inneres `Err`: der Transport scheiterte, R3 hielt aber; was danach **erlaubt** ist,
    ///   steht in [`Fehlschlag`]. Entscheiden und neu prüfen muss der Aufrufer.
    ///
    /// Der Verbrauch eines ausgeführten Versuchs wird auch bei verworfenem Ergebnis gezählt.
    pub(super) async fn versuch<T, F, Fut>(
        &mut self,
        senden: F,
    ) -> Result<Result<T, Fehlschlag>, VersandFehler>
    where
        F: FnOnce(Versandfreigabe) -> Fut,
        Fut: Future<Output = Versuch<T>>,
    {
        let geprueft = match self.pruefung {
            Pruefung::Voll(quelle) => freigabe_ausstellen(quelle, self.ziel, self.quellen)?,
            Pruefung::Uebergang => freigabe_uebergang(self.ziel),
        };
        self.regelwerk_und_budget()?;

        let Geprueft { freigabe, r2 } = geprueft;
        let versuch = senden(freigabe).await;
        self.ausgefuehrt = true;
        if let Ok((_, verbrauch)) = &versuch {
            self.tokens.prompt_tokens += verbrauch.prompt_tokens;
            self.tokens.completion_tokens += verbrauch.completion_tokens;
        }
        let versuch = match self.pruefung {
            // R3 in jedem Fall, auch nach einem Transportfehler; bei Abweichung ist das ganze
            // Ergebnis verworfen.
            Pruefung::Voll(quelle) => abschliessen(quelle, r2.as_ref(), Ok(versuch))?,
            Pruefung::Uebergang => versuch,
        };
        Ok(versuch.map(|(wert, _)| wert))
    }

    /// Regelwerk vor dem Budget: ein gesperrter Anbieter bekommt gar nichts. Beide bei **jedem**
    /// Versuch; die Verweigerung bleibt ungebucht.
    fn regelwerk_und_budget(&self) -> Result<(), VersandFehler> {
        #[cfg(test)]
        if let Some(ergebnis) = testhaken::regelwerk(self.anbieter, self.ziel.modell()) {
            return ergebnis;
        }
        crate::managers::compliance::check_call(self.anbieter)
            .map_err(|meldung| VersandFehler::Compliance { meldung })?;
        usage::check_budget(self.anbieter, self.ziel.modell())
            .map_err(|meldung| VersandFehler::Budget { meldung })
    }

    /// Bucht den Aufruf, wenn mindestens ein Versuch ausgeführt wurde.
    pub(super) async fn buchen<T>(self, ergebnis: &Result<T, VersandFehler>) {
        if !self.ausgefuehrt {
            return;
        }
        let dauer = self.start.elapsed().as_millis().min(u32::MAX as u128) as u32;
        usage::record_call(
            self.zweck,
            self.anbieter,
            self.ziel.modell(),
            self.tokens,
            dauer,
            ergebnis
                .as_ref()
                .map(|_| ())
                .map_err(VersandFehler::alte_textform),
        )
        .await;
    }
}

/// Ein Stück Inhaltsausgabe in der Reihenfolge seines Eintreffens.
enum Ereignis {
    Text(String),
    Denken(String),
}

/// Der Zwischenspeicher eines Versuchs.
#[derive(Default)]
struct Puffer {
    ereignisse: Vec<Ereignis>,
    denken_bytes: usize,
    denken_verworfen: usize,
}

fn zurueckhalten(puffer: &Mutex<Puffer>, ereignis: Ereignis) {
    // Vergiftet die Sperre (ein Panik an anderer Stelle), geht die Ausgabe verloren: sicher.
    let Ok(mut puffer) = puffer.lock() else {
        return;
    };
    if let Ereignis::Denken(text) = &ereignis {
        if puffer.denken_bytes + text.len() > PUFFER_DENKEN_MAX {
            puffer.denken_verworfen += 1;
            return;
        }
        puffer.denken_bytes += text.len();
    }
    puffer.ereignisse.push(ereignis);
}

/// Die Ausgabewege eines Versuchs: direkt an die Rückrufe, oder zurückgehalten, bis der
/// Abschlussvergleich bestanden ist.
struct Versuchsausgabe<'a> {
    puffer: Option<Arc<Mutex<Puffer>>>,
    /// Was der Transport als `on_delta` bekommt.
    text: Box<dyn Fn(&str) + Send + Sync + 'a>,
    /// Was der Transport als Denksenke bekommt.
    denken: Option<DenkSenke>,
    /// Die echten Empfänger.
    on_delta: &'a (dyn Fn(&str) + Send + Sync),
    senke: Option<DenkSenke>,
}

impl<'a> Versuchsausgabe<'a> {
    fn neu(
        puffern: bool,
        on_delta: &'a (dyn Fn(&str) + Send + Sync),
        senke: Option<DenkSenke>,
    ) -> Self {
        if !puffern {
            return Self {
                puffer: None,
                text: Box::new(move |stueck| on_delta(stueck)),
                denken: senke.clone(),
                on_delta,
                senke,
            };
        }
        let puffer = Arc::new(Mutex::new(Puffer::default()));
        let fuer_text = Arc::clone(&puffer);
        let fuer_denken = Arc::clone(&puffer);
        // Ohne Empfänger für Denktexte gibt es nichts zurückzuhalten.
        let denken: Option<DenkSenke> = senke.as_ref().map(|_| {
            Arc::new(move |stueck: &str| {
                zurueckhalten(&fuer_denken, Ereignis::Denken(stueck.to_owned()))
            }) as DenkSenke
        });
        Self {
            puffer: Some(puffer),
            text: Box::new(move |stueck| {
                zurueckhalten(&fuer_text, Ereignis::Text(stueck.to_owned()))
            }),
            denken,
            on_delta,
            senke,
        }
    }

    /// Die Ausgabe für den Transport.
    fn ausgabe(&self) -> Ausgabe<'_> {
        Ausgabe {
            on_delta: &*self.text,
            senke: self.denken.clone(),
        }
    }

    /// Die Denksenke für Transporte, die nur diese brauchen (CLI).
    fn denksenke(&self) -> Option<DenkSenke> {
        self.denken.clone()
    }

    /// Gibt das Zurückgehaltene an die echten Empfänger, in der Reihenfolge des Eintreffens.
    /// Nur nach bestandenem Abschlussvergleich aufrufen. Ohne Puffer ist nichts zu tun.
    fn freigeben(self) {
        let Some(puffer) = self.puffer else {
            return;
        };
        let Ok(mut puffer) = puffer.lock() else {
            return;
        };
        if puffer.denken_verworfen > 0 {
            log::warn!(
                "Denktext-Puffer voll: {} Stücke verworfen",
                puffer.denken_verworfen
            );
        }
        for ereignis in std::mem::take(&mut puffer.ereignisse) {
            match ereignis {
                Ereignis::Text(stueck) => (self.on_delta)(&stueck),
                Ereignis::Denken(stueck) => {
                    if let Some(senke) = &self.senke {
                        senke(&stueck);
                    }
                }
            }
        }
    }
}

/// Der eine Einstieg für Modellaufrufe: prüft frisch an der Versandgrenze, sendet **einmal** an
/// das gebundene `ziel` und schließt mit dem Abschlussvergleich ab. Ablauf, Fallbacks, Puffer
/// und Buchung stehen in der Moduldoku.
///
/// `anfrage.anbieter` muss der Anbieter des `ziel` sein (gleiche ID); sonst wäre Compliance für
/// einen anderen Anbieter geprüft als der, an den gesendet wird: `Gesperrt`.
pub async fn versenden(
    quelle: &dyn FreigabeQuelle,
    ziel: &Ziel,
    quellen: &Quellen,
    anfrage: Anfrage<'_>,
) -> Result<Antwort, VersandFehler> {
    ausfuehren(Pruefung::Voll(quelle), ziel, quellen, anfrage).await
}

/// Gemeinsamer Kern von [`versenden`] und der Übergangsfassade.
pub(super) async fn ausfuehren(
    pruefung: Pruefung<'_>,
    ziel: &Ziel,
    quellen: &Quellen,
    anfrage: Anfrage<'_>,
) -> Result<Antwort, VersandFehler> {
    let Anfrage {
        zweck,
        anbieter,
        api_key,
        inhalt,
    } = anfrage;
    if ziel.anbieter_id() != anbieter.id {
        return Err(VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT));
    }
    let mut lauf = Lauf::neu(pruefung, ziel, quellen, anbieter, zweck);
    let ctx = Kontext {
        anbieter,
        api_key: &api_key,
        zweck,
    };
    let ergebnis = match inhalt {
        // Auf den Haufen (siehe `uebergang`): die Futures von Chat und Strom sind groß.
        Inhalt::Chat(chat_inhalt) => Box::pin(chat(&mut lauf, &ctx, &chat_inhalt)).await,
        Inhalt::Strom {
            nachrichten,
            grenzen,
            on_delta,
        } => Box::pin(strom(&mut lauf, &ctx, &nachrichten, &grenzen, on_delta)).await,
    };
    lauf.buchen(&ergebnis).await;
    ergebnis
}

/// Eine Antwort am Stück.
async fn chat(
    lauf: &mut Lauf<'_>,
    ctx: &Kontext<'_>,
    inhalt: &ChatInhalt,
) -> Result<Antwort, VersandFehler> {
    let (ziel, quellen) = (lauf.ziel, lauf.quellen);
    let antwort = match cli_chat::cli_des_ziels(ziel) {
        Some(which) => {
            let puffern = lauf.pruefung.puffert(ziel, quellen);
            // Eine Antwort am Stück hat keine Textstücke; nur Denkzeilen der Codex-CLI.
            let kein_empfaenger = |_: &str| {};
            Box::pin(cli_lauf(
                lauf,
                which,
                &inhalt.nutzer,
                inhalt.system.as_deref(),
                inhalt.schema.as_ref(),
                &kein_empfaenger,
                crate::llm_client::denksenke(),
                puffern,
            ))
            .await?
        }
        None => Box::pin(
            lauf.versuch(|freigabe| http::chat_einmal(freigabe, ziel, quellen, ctx, inhalt)),
        )
        .await?
        .map_err(Fehlschlag::endgueltig)?,
    };
    Ok(Antwort {
        text: antwort.content,
        abgeschnitten: antwort.truncated,
    })
}

/// Ein CLI-Chat samt dem Neuversuch nach dem Update einer zu alten Codex-CLI. Der Neuversuch
/// ist ein **neuer** Versuch mit eigener Prüfung; ein Update ändert nur die Programmdatei, nie
/// das gebundene Ziel.
#[allow(clippy::too_many_arguments)]
async fn cli_lauf(
    lauf: &mut Lauf<'_>,
    which: Cli,
    nutzer: &str,
    system: Option<&str>,
    schema: Option<&serde_json::Value>,
    on_delta: &(dyn Fn(&str) + Send + Sync),
    senke: Option<DenkSenke>,
    puffern: bool,
) -> Result<ChatReply, VersandFehler> {
    let (ziel, quellen) = (lauf.ziel, lauf.quellen);
    let mut aktualisiert = false;
    loop {
        let ausgabe = Versuchsausgabe::neu(puffern, on_delta, senke.clone());
        let denksenke = ausgabe.denksenke();
        // `?`: verworfenes Ergebnis oder Verweigerung, `ausgabe` geht ungenutzt unter.
        let ergebnis = Box::pin(lauf.versuch(|freigabe| {
            cli_versuch(
                freigabe, ziel, quellen, which, nutzer, system, schema, denksenke,
            )
        }))
        .await?;
        // Der Abschlussvergleich hat gehalten: die Ausgabe dieses Versuchs darf hinaus.
        ausgabe.freigeben();
        match ergebnis {
            Ok(antwort) => return Ok(antwort),
            Err(Fehlschlag::CliVeraltet) if !aktualisiert => {
                aktualisiert = true;
                if !codex_aktualisieren().await {
                    return Err(VersandFehler::CliUpdate);
                }
            }
            Err(anderer) => return Err(anderer.endgueltig()),
        }
    }
}

/// Der eine Transportversuch einer CLI. Unter `cfg(test)` kann ein Test ihn ersetzen
/// ([`testhaken`]), ohne Prozess; die Freigabe wird auch dann eingelöst.
#[allow(clippy::too_many_arguments)]
async fn cli_versuch(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    which: Cli,
    nutzer: &str,
    system: Option<&str>,
    schema: Option<&serde_json::Value>,
    senke: Option<DenkSenke>,
) -> Versuch<ChatReply> {
    #[cfg(test)]
    let freigabe = match testhaken::cli_versuch(freigabe, ziel, quellen, senke.clone()) {
        Ok(ergebnis) => return ergebnis,
        Err(freigabe) => freigabe,
    };
    cli_chat::einmal(
        freigabe, ziel, quellen, which, nutzer, system, schema, senke,
    )
    .await
}

/// Aktualisiert die Codex-CLI (blockiert bis zu fünf Minuten, daher auf einem Blocking-Thread).
/// `true`: ein neuer Versuch lohnt. Unter `cfg(test)` ersetzbar ([`testhaken`]).
async fn codex_aktualisieren() -> bool {
    #[cfg(test)]
    if let Some(ergebnis) = testhaken::aktualisieren() {
        return ergebnis;
    }
    tokio::task::spawn_blocking(cli_update::ensure_codex_updated)
        .await
        .unwrap_or(false)
}

/// Eine Antwort als Strom.
async fn strom(
    lauf: &mut Lauf<'_>,
    ctx: &Kontext<'_>,
    nachrichten: &[StreamMessage],
    grenzen: &StreamLimits,
    on_delta: &(dyn Fn(&str) + Send + Sync),
) -> Result<Antwort, VersandFehler> {
    let (ziel, quellen) = (lauf.ziel, lauf.quellen);
    let puffern = lauf.pruefung.puffert(ziel, quellen);
    let senke = crate::llm_client::denksenke();

    if let Some(which) = cli_chat::cli_des_ziels(ziel) {
        // Die CLI liefert am Stück: Systemteile als Systemtext, der Rest als Gespräch mit Rollen.
        let system: Vec<&str> = nachrichten
            .iter()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .collect();
        let gespraech: Vec<String> = nachrichten
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| dialog_line(&m.role, &m.content))
            .collect();
        let system = (!system.is_empty()).then(|| system.join("\n\n"));
        let gespraech = gespraech.join("\n\n");
        let aufruf = Box::pin(cli_lauf(
            lauf,
            which,
            &gespraech,
            system.as_deref(),
            None,
            on_delta,
            // Die Denksenke gibt es hier wie bisher nur für Codex (Zeilenabgriff im Transport).
            senke,
            puffern,
        ));
        let aufruf = CLI_REQUIRE_USAGE.scope(grenzen.strict, aufruf);
        let antwort = match grenzen.max_tokens {
            Some(n) => CLI_MAX_TOKENS.scope(n, aufruf).await?,
            None => aufruf.await?,
        };
        // Die Ausgabe des Versuchs ist schon freigegeben; der Text selbst folgt jetzt.
        let text = antwort.content.unwrap_or_default();
        on_delta(&text);
        return Ok(Antwort {
            text: Some(text),
            abgeschnitten: false,
        });
    }

    let erste = Versuchsausgabe::neu(puffern, on_delta, senke.clone());
    let ergebnis = {
        let ausgabe = erste.ausgabe();
        Box::pin(lauf.versuch(|freigabe| {
            http::strom(freigabe, ziel, quellen, ctx, nachrichten, grenzen, &ausgabe)
        }))
        .await?
    };
    erste.freigeben();
    match ergebnis {
        Ok(text) => Ok(strom_antwort(text)),
        Err(Fehlschlag::OhneStromNeu) => {
            log::info!("Versand: neuer Versuch ohne Streaming");
            let zweite = Versuchsausgabe::neu(puffern, on_delta, senke);
            let ergebnis = {
                let ausgabe = zweite.ausgabe();
                Box::pin(lauf.versuch(|freigabe| {
                    http::ohne_strom(freigabe, ziel, quellen, ctx, nachrichten, grenzen, &ausgabe)
                }))
                .await?
            };
            zweite.freigeben();
            ergebnis.map(strom_antwort).map_err(Fehlschlag::endgueltig)
        }
        Err(anderer) => Err(anderer.endgueltig()),
    }
}

fn strom_antwort(text: String) -> Antwort {
    Antwort {
        text: Some(text),
        abgeschnitten: false,
    }
}

/// Rahmen für Wiederholungen **durch den Aufrufer** (JSON-Reparatur, vorübergehende Fehler):
/// ruft `aufruf(n)` (n zählt ab 0) höchstens `versuche`-mal. Jeder Aufruf soll ein vollständiges
/// [`versenden`] sein, also **ein neuer Versuch mit eigener Prüfung**; der Rahmen hält keine
/// Freigabe und kein Ergebnis zurück.
///
/// Wiederholt wird nur bei [`VersandFehler::ist_wiederholbar`]. [`VersandFehler::Gesperrt`]
/// (terminal), Abbruch, Budget, Compliance und Kontextüberlauf beenden die Schleife sofort mit
/// diesem Fehler; ein Aufrufer, der nach `Gesperrt` den Inhalt zerlegt, repariert oder
/// umleitet, verletzt den Vertrag.
pub async fn mit_wiederholung<T, F, Fut>(versuche: usize, mut aufruf: F) -> Result<T, VersandFehler>
where
    F: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, VersandFehler>>,
{
    let versuche = versuche.max(1);
    let mut nr = 0;
    loop {
        match aufruf(nr).await {
            Ok(wert) => return Ok(wert),
            Err(fehler) => {
                nr += 1;
                if nr >= versuche || !fehler.ist_wiederholbar() {
                    return Err(fehler);
                }
                log::info!("Versand: Wiederholung {nr} nach {}", fehler.code());
            }
        }
    }
}

#[cfg(test)]
#[path = "versand_tests.rs"]
mod versand_tests;
