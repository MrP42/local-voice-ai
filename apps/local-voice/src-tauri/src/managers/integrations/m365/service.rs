//! Der Dienst des Microsoft-365-Kontos: Anmeldung, Zugriffstoken, Anfragen (A5).
//!
//! Wiederverwendet wird, was der Kalender (`calendar::graph`) schon kann und
//! getestet hat: PKCE, Loopback-Listener, Token-Endpunkt, Drosselung. Neu sind die
//! Scopes je Faehigkeit und die Ablage im Integrations-Namensraum.
//!
//! Ablauf eines Aufrufs (`send_authed`): Zugriffstoken aus dem Speicher oder per
//! Erneuerungs-Token holen, Anfrage senden; bei 401 EINMAL erneuern und wiederholen,
//! bei einem zweiten 401 oder einem ungueltigen Erneuerungs-Token ist das Konto auf
//! „neu anmelden“ (das tote Token wird geloescht).
//!
//! Nebenlaeufigkeit: Erneuern laeuft durch EIN Tor (`refresh_gate`); wer wartet,
//! nimmt das inzwischen erneuerte Zugriffstoken, statt selbst zu erneuern (zwei
//! gleichzeitige 401 ergeben einen Aufruf am Token-Endpunkt, nicht zwei, die sich
//! beim Schreiben des neuen Erneuerungs-Tokens ueberholen).
//!
//! Abmelden waehrend des Erneuerns (B22, QG5): Jedes Konto hat eine **Generation**
//! (`generations`). Anmelden, Abmelden und das Loeschen eines toten Tokens erhoehen sie, jeweils
//! unter derselben Sperre, unter der das Konto geschrieben oder geloescht wird. Ein Erneuern merkt
//! sich die Generation VOR dem Lesen des Kontos und legt sein Ergebnis (neues Erneuerungs-Token,
//! Zugriffstoken im Speicher) nur ab, wenn sie unter derselben Sperre noch stimmt. Sonst wird das
//! Ergebnis verworfen und mit dem aktuellen Stand neu begonnen: ein abgemeldetes Konto bleibt
//! abgemeldet (`NeedsSignIn`, kein Zugriff mit dem spaet gelieferten Token), ein neu angemeldetes
//! wird nicht mit dem Token des alten ueberschrieben.
//!
//! Sicherheit: Umleitungen werden nie verfolgt, das Zugriffstoken geht nur an den
//! Graph-Host aus den Endpunkten (nie an `uploadUrl` oder `@odata.nextLink`).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::Value;
use zeroize::Zeroizing;

use super::account::{StoredAccount, Vault};
use super::config::M365Config;
use super::error::M365Error;
use crate::managers::calendar::graph::{
    self, parse_retry_after, short_description, Endpoints, GraphState,
};
use crate::managers::integrations::model::{Capability, Integration, Kind};

/// Antworten werden nie groesser gelesen (Termine, Fehler, Upload-Antworten).
pub const MAX_REPLY_BYTES: usize = 8 * 1024 * 1024;
/// Zeitlimit einer gewoehnlichen Anfrage.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Zeitlimit einer Anfrage mit Dateiinhalt (ein Stueck oder eine kleine Datei).
pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);
/// So lange wartet die Anmeldung auf den Browser.
pub const SIGN_IN_TIMEOUT: Duration = graph::SIGN_IN_TIMEOUT;
/// Ein Zugriffstoken gilt im Speicher nur bis kurz vor seinem Ablauf.
const TOKEN_SAFETY_MARGIN: Duration = Duration::from_secs(120);

/// Eine Integration der Art `m365` mit ihrer gelesenen Konfiguration.
#[derive(Clone, Debug)]
pub struct Acct {
    pub integ: Integration,
    pub cfg: M365Config,
}

impl Acct {
    pub fn from_integration(integ: Integration) -> Result<Self, M365Error> {
        if integ.kind != Kind::M365 {
            return Err(M365Error::Invalid(
                "Das ist kein Microsoft-365-Konto.".to_string(),
            ));
        }
        let cfg = M365Config::from_json(&integ.config_json)?;
        Ok(Self { integ, cfg })
    }

    /// Ohne diese eingeschaltete Faehigkeit keine Aktion (zweite Sicherung neben dem
    /// Recht im Tor).
    pub fn require(&self, cap: Capability) -> Result<(), M365Error> {
        if self.cfg.has(cap) {
            Ok(())
        } else {
            Err(M365Error::CapabilityOff(cap))
        }
    }
}

/// Eine Antwort, gelesen und begrenzt.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub retry_after: Option<String>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Result<Value, M365Error> {
        serde_json::from_slice(&self.body)
            .map_err(|_| M365Error::Parse("keine gültige JSON-Antwort".to_string()))
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

struct Cached {
    scope: String,
    token: Zeroizing<String>,
    until: Instant,
}

pub struct M365Service {
    pub ep: Endpoints,
    pub vault: Vault,
    /// Laufende Anmeldung (nur eine gleichzeitig) und Drosselung.
    pub graph: GraphState,
    cache: Mutex<std::collections::HashMap<String, Cached>>,
    /// Kontogeneration je Integration (B22): siehe Moduldoku. Fehlt ein Eintrag, ist es 0.
    generations: Mutex<std::collections::HashMap<String, u64>>,
    refresh_gate: tokio::sync::Mutex<()>,
    /// Pause zwischen zwei Versuchen eines Upload-Stuecks (mal Versuchsnummer).
    pub retry_pause: Duration,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Ergebnis einer Anmeldung: Konto zur Anzeige.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedInfo {
    pub address: String,
    pub name: Option<String>,
}

impl M365Service {
    pub fn new(ep: Endpoints, vault: Vault) -> Self {
        Self {
            ep,
            vault,
            graph: GraphState::default(),
            cache: Mutex::new(Default::default()),
            generations: Mutex::new(Default::default()),
            refresh_gate: tokio::sync::Mutex::new(()),
            retry_pause: super::drive::DEFAULT_RETRY_PAUSE,
        }
    }

    pub fn production() -> Self {
        Self::new(Endpoints::production(), Vault::system())
    }

    // -- Zugriffstoken im Arbeitsspeicher ------------------------------------

    fn cache_get(&self, id: &str, scope: &str) -> Option<Zeroizing<String>> {
        let map = lock(&self.cache);
        map.get(id)
            .filter(|c| c.scope == scope && c.until > Instant::now())
            .map(|c| c.token.clone())
    }

    fn cache_put(&self, id: &str, scope: &str, token: Zeroizing<String>, expires_in: Duration) {
        let valid = expires_in.saturating_sub(TOKEN_SAFETY_MARGIN);
        lock(&self.cache).insert(
            id.to_string(),
            Cached {
                scope: scope.to_string(),
                token,
                until: Instant::now() + valid,
            },
        );
    }

    /// Vergisst das Zugriffstoken einer Integration (Abmelden, widerrufen, ersetzt).
    pub fn forget(&self, id: &str) {
        lock(&self.cache).remove(id);
    }

    // -- Anmeldung -----------------------------------------------------------

    /// Interaktive Anmeldung mit genau den Scopes der eingeschalteten Faehigkeiten.
    /// `open` oeffnet die Adresse im Systembrowser (in Tests ein „Browser“ ohne
    /// Fenster). Das Konto wird erst NACH Token und Profil geschrieben, atomar; bei
    /// jedem Fehler davor bleibt der alte Stand (altes Token oder keins).
    pub async fn sign_in(
        &self,
        a: &Acct,
        open: impl FnOnce(String) -> Result<(), String>,
        timeout: Duration,
    ) -> Result<SignedInfo, M365Error> {
        if a.cfg.client_id.is_empty() {
            return Err(M365Error::NotConfigured);
        }
        if a.cfg.capabilities.is_empty() {
            return Err(M365Error::NoCapability);
        }
        let (cancel, _active) = self.graph.begin_sign_in()?;
        let scope = a.cfg.scope_string();
        let signed = graph::sign_in_scoped(
            &self.ep,
            &a.cfg.client_id,
            &a.cfg.tenant,
            &scope,
            open,
            timeout,
            &cancel,
        )
        .await?;
        let me = graph::fetch_me(&self.ep, &signed.access_token).await?;
        let account = StoredAccount::new(
            a.cfg.client_id.clone(),
            a.cfg.tenant.clone(),
            scope.clone(),
            signed.account.refresh_token().to_string(),
            Some(me.address.clone()),
            me.name.clone(),
        );
        self.install_account(&a.integ, &account)
            .map_err(M365Error::Store)?;
        self.cache_put(
            &a.integ.id,
            &scope,
            signed.access_token.clone(),
            signed.expires_in,
        );
        Ok(SignedInfo {
            address: me.address,
            name: me.name,
        })
    }

    /// Die aktuelle Generation des Kontos (siehe Moduldoku).
    fn generation(&self, id: &str) -> u64 {
        lock(&self.generations).get(id).copied().unwrap_or(0)
    }

    /// Legt das Konto einer neuen Anmeldung ab und beginnt eine neue Generation: ein gerade
    /// laufendes Erneuern des alten Kontos darf es nicht mehr ueberschreiben. Scheitert das
    /// Schreiben, bleibt alles wie es war (auch die Generation).
    pub(super) fn install_account(
        &self,
        i: &Integration,
        account: &StoredAccount,
    ) -> Result<(), String> {
        let mut generations = lock(&self.generations);
        self.vault.save(i, account)?;
        *generations.entry(i.id.clone()).or_insert(0) += 1;
        // Das Zugriffstoken des alten Kontos gilt nicht fuer das neue.
        self.forget(&i.id);
        Ok(())
    }

    /// Bricht eine laufende Anmeldung ab; `false`, wenn keine laeuft.
    pub fn cancel_sign_in(&self) -> bool {
        self.graph.cancel_sign_in()
    }

    /// Laeuft gerade eine Anmeldung?
    pub fn signing_in(&self) -> bool {
        self.graph.is_signing_in()
    }

    /// Abmelden: Token loeschen, Zugriffstoken vergessen. Die Integration bleibt. Die neue
    /// Generation macht ein gerade laufendes Erneuern wirkungslos (B22).
    pub fn sign_out(&self, a: &Integration) {
        let mut generations = lock(&self.generations);
        *generations.entry(a.id.clone()).or_insert(0) += 1;
        self.vault.clear(a);
        self.forget(&a.id);
    }

    // -- Zugriffstoken -------------------------------------------------------

    /// Ein gueltiges Zugriffstoken fuer genau die Scopes der eingeschalteten
    /// Faehigkeiten. `stale`: das Token, mit dem gerade ein 401 kam; es wird nicht
    /// noch einmal herausgegeben.
    pub async fn access_token(
        &self,
        a: &Acct,
        stale: Option<&str>,
    ) -> Result<Zeroizing<String>, M365Error> {
        let required = a.cfg.required_scopes();
        let scope = required.join(" ");
        self.load_checked(a, &required)?;
        if stale.is_none() {
            if let Some(token) = self.cache_get(&a.integ.id, &scope) {
                return Ok(token);
            }
        }
        let _gate = self.refresh_gate.lock().await;
        // Hoechstens zwei Runden: wurde das Konto waehrend des Erneuerns abgemeldet oder ersetzt,
        // wird das Ergebnis verworfen und mit dem aktuellen Stand neu begonnen (B22).
        for _round in 0..2 {
            // Wer vor uns erneuert hat, hat das Token schon in den Speicher gelegt.
            if let Some(token) = self.cache_get(&a.integ.id, &scope) {
                if Some(token.as_str()) != stale {
                    return Ok(token);
                }
            }
            // Erst die Generation, dann das Konto: ein Wechsel dazwischen faellt beim Ablegen auf.
            let generation = self.generation(&a.integ.id);
            // Das Konto neu lesen: der Vorgaenger kann das Erneuerungs-Token ersetzt haben.
            let account = self.load_checked(a, &required)?;
            let tokens = match graph::refresh_tokens_scoped(
                &self.ep,
                &account.client_id,
                &account.tenant,
                account.refresh_token(),
                &scope,
            )
            .await
            {
                Ok(t) => t,
                Err(graph::GraphError::NeedsSignIn) => {
                    let mut generations = lock(&self.generations);
                    if generations.get(&a.integ.id).copied().unwrap_or(0) == generation {
                        // Microsoft hat das Token widerrufen oder es ist abgelaufen: es ist tot.
                        *generations.entry(a.integ.id.clone()).or_insert(0) += 1;
                        self.vault.clear(&a.integ);
                        self.forget(&a.integ.id);
                        return Err(M365Error::NeedsSignIn);
                    }
                    // Das Konto ist inzwischen ein anderes (neu angemeldet) oder weg: das tote
                    // Token gehoerte dem alten, das aktuelle bleibt unangetastet.
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            let generations = lock(&self.generations);
            if generations.get(&a.integ.id).copied().unwrap_or(0) != generation {
                // Abgemeldet oder neu angemeldet, waehrend Microsoft antwortete: nichts ablegen,
                // kein Zugriffstoken herausgeben.
                continue;
            }
            if let Some(new) = &tokens.refresh_token {
                if new.as_str() != account.refresh_token() {
                    let updated = account.with_refresh_token(new.to_string());
                    // Scheitert das Schreiben (Platte voll), laeuft dieser Aufruf mit dem
                    // Zugriffstoken weiter und das alte Erneuerungs-Token bleibt gueltig.
                    if let Err(e) = self.vault.save(&a.integ, &updated) {
                        log::warn!("m365: neues Erneuerungs-Token nicht gespeichert: {e}");
                    }
                }
            }
            self.cache_put(
                &a.integ.id,
                &scope,
                tokens.access_token.clone(),
                tokens.expires_in,
            );
            return Ok(tokens.access_token.clone());
        }
        Err(M365Error::NeedsSignIn)
    }

    /// Das Konto aus dem Geheimnisspeicher; fehlt es, ist es kaputt oder passt es nicht
    /// zur Konfiguration, ist „neu anmelden“ noetig; fehlen Scopes, „zustimmen“.
    fn load_checked(&self, a: &Acct, required: &[String]) -> Result<StoredAccount, M365Error> {
        let account = match self.vault.load(&a.integ) {
            Ok(Some(acc)) => acc,
            Ok(None) | Err(_) => return Err(M365Error::NeedsSignIn),
        };
        if a.cfg.client_id.is_empty() {
            return Err(M365Error::NotConfigured);
        }
        if account.client_id != a.cfg.client_id || account.tenant != a.cfg.tenant {
            return Err(M365Error::NeedsSignIn);
        }
        let missing = account.missing(required);
        if !missing.is_empty() {
            return Err(M365Error::NeedsConsent { missing });
        }
        Ok(account)
    }

    // -- Anfragen ------------------------------------------------------------

    fn build_client(&self) -> Result<reqwest::Client, M365Error> {
        let mut builder = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // Keine Umleitung verfolgen: das Zugriffstoken darf nirgends anders hin.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!(
                "LocalVoiceAI/",
                env!("CARGO_PKG_VERSION"),
                " (m365)"
            ));
        if !self.ep.use_env_proxy {
            builder = builder.no_proxy();
        }
        builder
            .build()
            .map_err(|e| M365Error::Network(e.without_url().to_string()))
    }

    pub(super) fn graph_base(&self) -> &str {
        self.ep.graph.trim_end_matches('/')
    }

    /// Eine Anfrage an Graph mit Zugriffstoken; bei 401 einmal erneuern und
    /// wiederholen. `retry_safe`: darf bei unklarem Ausgang als „nicht ausgefuehrt“
    /// gelten (Lesen, Hochladen eines Stuecks); fuer `sendMail` `false`.
    pub(super) async fn send_authed(
        &self,
        a: &Acct,
        retry_safe: bool,
        build: impl Fn(&reqwest::Client, &str) -> reqwest::RequestBuilder,
    ) -> Result<Reply, M365Error> {
        let client = self.build_client()?;
        let token = self.access_token(a, None).await?;
        let first = send_once(build(&client, &token), retry_safe).await?;
        if first.status != 401 {
            return Ok(first);
        }
        let fresh = self.access_token(a, Some(token.as_str())).await?;
        let second = send_once(build(&client, &fresh), retry_safe).await?;
        if second.status == 401 {
            // Frisches Token und trotzdem 401: Konto, Mandant oder Zustimmung stimmen nicht.
            self.forget(&a.integ.id);
            return Err(M365Error::NeedsSignIn);
        }
        Ok(second)
    }

    /// Name und Adresse des angemeldeten Kontos (`GET /me`): der Test der Verbindung.
    /// Braucht keine eingeschaltete Faehigkeit, nur `User.Read` der Anmeldung.
    pub async fn me(&self, a: &Acct) -> Result<SignedInfo, M365Error> {
        let url = format!(
            "{}/me?$select=displayName,mail,userPrincipalName",
            self.graph_base()
        );
        let reply = self
            .send_authed(a, true, |c, token| c.get(&url).bearer_auth(token))
            .await?;
        if !reply.is_success() {
            return Err(error_of(&reply, "Konto"));
        }
        let v = reply.json()?;
        let text = |key: &str| {
            v.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
        };
        let address = text("mail")
            .filter(|m| m.contains('@'))
            .or_else(|| text("userPrincipalName").filter(|m| m.contains('@')))
            .map(str::to_lowercase)
            .ok_or_else(|| M365Error::Parse("Konto ohne E-Mail-Adresse".to_string()))?;
        Ok(SignedInfo {
            address,
            name: text("displayName").map(str::to_string),
        })
    }

    /// Eine Anfrage OHNE Zugriffstoken (die `uploadUrl` einer Upload-Sitzung ist
    /// vorab beglaubigt; ein Token dort hinzuschicken wuerde es weitergeben).
    pub(super) async fn send_plain(
        &self,
        retry_safe: bool,
        build: impl FnOnce(&reqwest::Client) -> reqwest::RequestBuilder,
    ) -> Result<Reply, M365Error> {
        let client = self.build_client()?;
        send_once(build(&client), retry_safe).await
    }
}

/// Sendet und liest die Antwort (begrenzt). Fehler der Verbindung: vor dem Senden
/// (`is_connect`) ist sicher nichts angekommen; danach ist der Ausgang bei
/// nicht wiederholbaren Aktionen unklar (`Uncertain`).
async fn send_once(rb: reqwest::RequestBuilder, retry_safe: bool) -> Result<Reply, M365Error> {
    let resp = rb.send().await.map_err(|e| classify(e, retry_safe))?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body = graph::read_limited(resp, MAX_REPLY_BYTES)
        .await
        .map_err(|e| match e {
            graph::GraphError::Network(m) if !retry_safe => M365Error::Uncertain(m),
            other => other.into(),
        })?;
    Ok(Reply {
        status,
        retry_after,
        body,
    })
}

fn classify(e: reqwest::Error, retry_safe: bool) -> M365Error {
    let e = e.without_url();
    if e.is_connect() {
        return M365Error::Network(e.to_string());
    }
    if e.is_timeout() {
        return if retry_safe {
            M365Error::Timeout
        } else {
            M365Error::Uncertain("Zeitüberschreitung".to_string())
        };
    }
    if retry_safe {
        M365Error::Network(e.to_string())
    } else {
        M365Error::Uncertain(e.to_string())
    }
}

/// Microsoft-Fehlercode und kurze Beschreibung aus `{"error":{"code","message"}}`.
/// Der Code besteht nur aus Buchstaben, Ziffern und `_`; Beschreibung auf eine
/// Zeile gekuerzt.
pub(super) fn graph_error_parts(body: &[u8]) -> (String, String) {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let code: String = v
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(60)
        .collect();
    let message = short_description(
        v.pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    (code, message)
}

/// Macht aus einer Antwort ohne Erfolg den passenden Fehler.
pub(super) fn error_of(reply: &Reply, what: &str) -> M365Error {
    let (code, message) = graph_error_parts(&reply.body);
    match reply.status {
        401 => M365Error::NeedsSignIn,
        403 => M365Error::Denied(if message.is_empty() {
            format!("HTTP 403 ({what})")
        } else {
            message
        }),
        404 => M365Error::NotFound(what.to_string()),
        429 => M365Error::Throttled {
            retry_after: parse_retry_after(reply.retry_after.as_deref(), chrono::Utc::now()),
        },
        503 if reply.retry_after.is_some() => M365Error::Throttled {
            retry_after: parse_retry_after(reply.retry_after.as_deref(), chrono::Utc::now()),
        },
        507 => M365Error::StorageFull,
        _ if code == "quotaLimitReached" => M365Error::StorageFull,
        status => M365Error::Http { status, code },
    }
}
