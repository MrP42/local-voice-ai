//! Nähte für Tests des Versands (lva#604, Teilpaket 3). Nur unter `cfg(test)`.
//!
//! Vier Stellen lassen sich je Test-Thread ersetzen, weil sie sonst globalen Zustand (Einstellungen,
//! Verbrauchsbuch), Prozesse (CLI, npm) oder echtes Netz (Cloud-Ziele) bräuchten:
//!
//! - **Regelwerk und Budget** ([`setze_regelwerk`]): zählt die Prüfungen je Versuch und kann sie
//!   verweigern. Die echten Prüfungen (`compliance::check_call`, `usage::check_budget`) laufen,
//!   solange kein Haken gesetzt ist.
//! - **CLI-Versuch** ([`setze_cli`]): ersetzt den Prozess durch einen Skriptwert je Versuch.
//! - **Update der Codex-CLI** ([`setze_aktualisierer`]).
//! - **Cloud-Proxy** ([`setze_cloud_proxy`]): Cloud-Ziele (kein Loopback) gehen an einen
//!   Test-Server, ohne TLS und ohne Umgebungsvariablen.
//!
//! Die Haken gelten für den **Test-Thread** (`thread_local`); `#[tokio::test]` läuft auf einem
//! einzigen Thread, die Haken sind also für genau diesen Test wirksam und für keinen anderen.
//! [`wache`] setzt am Ende des Tests alles zurück.

use std::cell::RefCell;
use std::rc::Rc;

use super::fehler::VersandFehler;
use super::pruefer::Versandfreigabe;
use super::quellen::Quellen;
use super::transport::{Fehlschlag, Versuch};
use super::ziel::Ziel;
use crate::llm_client::{ChatReply, DenkSenke};
use crate::settings::PostProcessProvider;

/// Prüfart (`"compliance"` oder `"budget"`), Anbieter, Modell: `Err` ist die Meldung der
/// Verweigerung.
type Regelwerk = Rc<dyn Fn(&'static str, &PostProcessProvider, &str) -> Result<(), String>>;
/// Nummer des Versuchs (ab 0) und die Denksenke, die der Transport bekäme.
type CliHaken = Rc<dyn Fn(usize, Option<DenkSenke>) -> Versuch<ChatReply>>;

#[derive(Default)]
struct Haken {
    regelwerk: Option<Regelwerk>,
    cli: Option<CliHaken>,
    cli_versuche: usize,
    aktualisierer: Option<Rc<dyn Fn() -> bool>>,
    cloud_proxy: Option<String>,
}

thread_local! {
    static HAKEN: RefCell<Haken> = RefCell::new(Haken::default());
}

/// Setzt beim Fallenlassen alle Haken dieses Threads zurück.
pub(in crate::llm_versand) struct Wache;

impl Drop for Wache {
    fn drop(&mut self) {
        HAKEN.with(|haken| *haken.borrow_mut() = Haken::default());
    }
}

/// Am Anfang eines Tests, der Haken setzt: `let _wache = testhaken::wache();`.
pub(in crate::llm_versand) fn wache() -> Wache {
    HAKEN.with(|haken| *haken.borrow_mut() = Haken::default());
    Wache
}

pub(in crate::llm_versand) fn setze_regelwerk(
    haken: impl Fn(&'static str, &PostProcessProvider, &str) -> Result<(), String> + 'static,
) {
    HAKEN.with(|h| h.borrow_mut().regelwerk = Some(Rc::new(haken)));
}

pub(in crate::llm_versand) fn setze_cli(
    haken: impl Fn(usize, Option<DenkSenke>) -> Versuch<ChatReply> + 'static,
) {
    HAKEN.with(|h| {
        let mut h = h.borrow_mut();
        h.cli = Some(Rc::new(haken));
        h.cli_versuche = 0;
    });
}

pub(in crate::llm_versand) fn setze_aktualisierer(haken: impl Fn() -> bool + 'static) {
    HAKEN.with(|h| h.borrow_mut().aktualisierer = Some(Rc::new(haken)));
}

pub(in crate::llm_versand) fn setze_cloud_proxy(adresse: &str) {
    HAKEN.with(|h| h.borrow_mut().cloud_proxy = Some(adresse.to_owned()));
}

/// Wie viele CLI-Versuche der Haken schon bedient hat.
pub(in crate::llm_versand) fn cli_versuche() -> usize {
    HAKEN.with(|h| h.borrow().cli_versuche)
}

/// Für `Lauf::regelwerk_und_budget`: `None`, solange kein Haken gesetzt ist (dann laufen die
/// echten Prüfungen).
pub(in crate::llm_versand) fn regelwerk(
    anbieter: &PostProcessProvider,
    modell: &str,
) -> Option<Result<(), VersandFehler>> {
    let haken = HAKEN.with(|h| h.borrow().regelwerk.clone())?;
    Some(
        haken("compliance", anbieter, modell)
            .map_err(|meldung| VersandFehler::Compliance { meldung })
            .and_then(|()| {
                haken("budget", anbieter, modell)
                    .map_err(|meldung| VersandFehler::Budget { meldung })
            }),
    )
}

/// Für den CLI-Versuch: ohne Haken kommt die Freigabe unverbraucht zurück (dann läuft der echte
/// Transport). Mit Haken wird sie eingelöst und der Skriptwert geliefert.
pub(in crate::llm_versand) fn cli_versuch(
    freigabe: Versandfreigabe,
    ziel: &Ziel,
    quellen: &Quellen,
    senke: Option<DenkSenke>,
) -> Result<Versuch<ChatReply>, Versandfreigabe> {
    let Some(haken) = HAKEN.with(|h| h.borrow().cli.clone()) else {
        return Err(freigabe);
    };
    if let Err(fehler) = freigabe.einloesen(ziel, quellen) {
        return Ok(Err(Fehlschlag::Fehler(fehler)));
    }
    let nr = HAKEN.with(|h| {
        let mut h = h.borrow_mut();
        let nr = h.cli_versuche;
        h.cli_versuche += 1;
        nr
    });
    Ok(haken(nr, senke))
}

/// Für das Update der Codex-CLI: `None` ohne Haken (dann läuft das echte Update).
pub(in crate::llm_versand) fn aktualisieren() -> Option<bool> {
    let haken = HAKEN.with(|h| h.borrow().aktualisierer.clone())?;
    Some(haken())
}

/// Für `http::client`: der Test-Proxy der Cloud-Ziele, falls gesetzt.
pub(in crate::llm_versand) fn cloud_proxy() -> Option<String> {
    HAKEN.with(|h| h.borrow().cloud_proxy.clone())
}
