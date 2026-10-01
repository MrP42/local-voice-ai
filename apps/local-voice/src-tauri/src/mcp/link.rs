//! Die Verbindung des MCP-Proxys zur laufenden App (A8): schreibende Werkzeuge laufen nicht in
//! diesem Prozess, sondern ueber die Named Pipe der Agentenbruecke (A7). Der Proxy ist nur ein
//! Uebersetzer von MCP (JSON-RPC ueber stdio) auf das Zeilenprotokoll der Bruecke; ob ein Werkzeug
//! erlaubt ist, entscheidet die App (Rechte je Werkzeug, Freigabe, Audit), nie der Proxy.
//!
//! - **Token**: aus `LVA_AGENT_TOKEN` (stdio-Transport: Zugangsdaten kommen aus der Umgebung, nicht
//!   aus Argumenten). Es wird nie ausgegeben, nie in Fehlertexte eingesetzt und nie protokolliert.
//! - **Eine Verbindung je Anfrage**: der Proxy haelt keine Pipe offen. Startet die App neu oder
//!   wird ein Zugang zurueckgezogen, gilt das bei der naechsten Anfrage, ohne Neustart des Proxys.
//! - **Fristen**: jede Anfrage laeuft in einem Arbeitsthread mit Gesamtfrist; haengt die App, kommt
//!   ein Hinweis statt eines Wartezustands ohne Ende.
//! - **Fehler sind Werkzeugfehler** (`isError`), keine Protokollfehler: ein Agent kann den Text
//!   lesen und dem Nutzer sagen, was zu tun ist. Fehlt das Token oder laeuft die App nicht, fehlen
//!   die schreibenden Werkzeuge in `tools/list`.

use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::agent_bridge::catalog::{self, STATUS_TOOL};
use crate::agent_bridge::client::{Client, ClientError};
use crate::agent_bridge::ctl::TOKEN_ENV;
use crate::agent_bridge::protocol::code;
use crate::agent_bridge::{ctl, pipe};

/// Frist fuer `tools/list` (Verbinden, Anmelden, Liste).
pub const LIST_DEADLINE: Duration = Duration::from_secs(15);
/// Frist fuer `tools/call`: die App wartet bis 30 s auf eine Freigabe, ein Vorlesen kann
/// Minuten dauern.
pub const CALL_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// Ist `name` ein Werkzeug, das ueber die Bruecke laeuft (Katalog oder Statusabfrage)?
pub fn is_bridge_tool(name: &str) -> bool {
    name == STATUS_TOOL || catalog::find(name).is_some()
}

/// Was ein Aufruf geliefert hat.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkReply {
    /// Ausgefuehrt; das Ergebnis des Werkzeugs.
    Done(Value),
    /// Die App wartet auf die Freigabe des Nutzers: kein Fehler, ein Hinweis.
    Pending {
        approval_id: String,
        message: String,
    },
}

/// Warum die App nichts geliefert hat. `text` ist Klartext fuer den Agenten, ohne Geheimnisse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkError {
    pub kind: LinkErrorKind,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkErrorKind {
    /// Kein Token in der Umgebung.
    NoToken,
    /// Die App laeuft nicht oder die Pipe ist nicht erreichbar.
    NotRunning,
    /// Token ungueltig oder zurueckgezogen.
    Auth,
    /// Das Werkzeug ist fuer diesen Zugang aus oder wurde abgelehnt.
    Denied,
    /// Die Frist ist abgelaufen.
    Timeout,
    /// Alles andere (Werkzeug gescheitert, Protokoll, Grenzen).
    Failed,
}

impl LinkError {
    fn new(kind: LinkErrorKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// Die Gegenstelle des Proxys (in Tests ersetzbar).
pub trait BridgeLink: Send + Sync {
    /// Die Werkzeuge, die dieser Zugang jetzt benutzen darf, als MCP-Definitionen.
    fn tools(&self) -> Result<Vec<Value>, LinkError>;
    /// Ruft ein Werkzeug. `approval_id`: der Agent kommt mit der Freigabe einer frueheren
    /// Antwort `pending` zurueck (dieselben Argumente).
    fn call(
        &self,
        name: &str,
        args: &Value,
        approval_id: Option<&str>,
    ) -> Result<LinkReply, LinkError>;
}

/// Name des zusaetzlichen Arguments, mit dem ein Agent eine erteilte Freigabe einloest.
pub const APPROVAL_ARG: &str = "approval_id";

/// Uebersetzt einen Fehler der Bruecke in einen Hinweis fuer den Agenten.
pub fn explain(e: &ClientError) -> LinkError {
    match e {
        ClientError::NotRunning(_) | ClientError::UntrustedServer(_) => LinkError::new(
            LinkErrorKind::NotRunning,
            "Local Voice AI läuft nicht (oder ist nicht erreichbar). Die schreibenden Werkzeuge gibt es nur, solange die App läuft: bitte die App starten und es erneut versuchen.",
        ),
        ClientError::Transport(_) => LinkError::new(
            LinkErrorKind::NotRunning,
            "Die Verbindung zu Local Voice AI wurde unterbrochen (die App wurde vielleicht beendet). Bitte erneut versuchen.",
        ),
        ClientError::Protocol(m) => LinkError::new(LinkErrorKind::Failed, m.clone()),
        ClientError::Remote { code: c, message, .. } => match c.as_str() {
            code::TOKEN_INVALID | code::TOKEN_REVOKED | code::UNAUTHENTICATED => LinkError::new(
                LinkErrorKind::Auth,
                format!(
                    "Der Zugang wurde nicht angenommen: {message} Den Schlüssel in der Umgebungsvariable {TOKEN_ENV} prüfen; in Local Voice AI unter Integrationen lässt sich ein neuer Zugang anlegen."
                ),
            ),
            code::TOOL_OFF | code::DENIED | code::APPROVAL_DENIED => LinkError::new(
                LinkErrorKind::Denied,
                format!("Nicht erlaubt: {message}"),
            ),
            _ => LinkError::new(LinkErrorKind::Failed, message.clone()),
        },
    }
}

/// Die Bruecke ueber die Named Pipe, mit dem Token des Zugangs.
pub struct PipeLink {
    token: String,
    pipe_name: Result<String, String>,
}

impl PipeLink {
    /// Aus der Umgebung (`LVA_AGENT_TOKEN`, `LVA_AGENT_PIPE`); `None`, wenn kein Token da ist.
    pub fn from_env() -> Option<Self> {
        let token = ctl::read_token(None, std::env::var(TOKEN_ENV).ok())
            .ok()
            .flatten()?;
        Some(Self {
            token,
            pipe_name: pipe::effective_pipe_name().map_err(|e| e.to_string()),
        })
    }

    #[cfg(test)]
    pub fn with(token: &str, pipe_name: &str) -> Self {
        Self {
            token: token.to_string(),
            pipe_name: Ok(pipe_name.to_string()),
        }
    }

    /// Verbindet und meldet an: eine frische Sitzung.
    fn open(&self) -> Result<impl Session, LinkError> {
        let name = self
            .pipe_name
            .as_ref()
            .map_err(|m| LinkError::new(LinkErrorKind::Failed, m.clone()))?;
        let mut client = Client::connect(name).map_err(|e| explain(&e))?;
        client.hello(Some(&self.token)).map_err(|e| explain(&e))?;
        Ok(client)
    }

    /// Fuehrt `work` in einem Arbeitsthread aus, hoechstens `deadline` lang.
    fn bounded<T: Send + 'static>(
        &self,
        deadline: Duration,
        work: impl FnOnce(&mut dyn Session) -> Result<T, LinkError> + Send + 'static,
    ) -> Result<T, LinkError> {
        let name = self.pipe_name.clone();
        let token = self.token.clone();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let link = PipeLink {
                token,
                pipe_name: name,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut session = link.open()?;
                work(&mut session)
            }));
            let _ = tx.send(result.unwrap_or_else(|_| {
                Err(LinkError::new(
                    LinkErrorKind::Failed,
                    "Interner Fehler im MCP-Proxy.",
                ))
            }));
        });
        rx.recv_timeout(deadline).unwrap_or_else(|_| {
            Err(LinkError::new(
                LinkErrorKind::Timeout,
                "Zeitüberschreitung: Local Voice AI hat nicht rechtzeitig geantwortet. Der Auftrag kann in der App trotzdem laufen; in der App unter Integrationen nachsehen.",
            ))
        })
    }
}

/// Eine angemeldete Sitzung (der echte `Client` oder, in Tests, eine Attrappe).
pub trait Session {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, ClientError>;
}

impl<R: std::io::BufRead, W: std::io::Write> Session for Client<R, W> {
    fn request(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        Client::request(self, method, params)
    }
}

/// Eine Werkzeugdefinition der Bruecke als MCP-Werkzeug: ohne das Feld `mode`.
pub fn to_mcp_tool(t: &Value) -> Option<Value> {
    let name = t.get("name")?.as_str()?;
    if !is_bridge_tool(name) {
        return None;
    }
    let mut schema = t
        .get("inputSchema")
        .cloned()
        .unwrap_or_else(|| json!({"type": "object"}));
    // Wer eine erteilte Freigabe einloest, braucht dafuer ein Argument: die MCP-Clients pruefen die
    // Eingaben gegen das Schema und wuerden ein unbekanntes Feld sonst verwerfen.
    if name != STATUS_TOOL {
        if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) {
            props.insert(
                APPROVAL_ARG.to_string(),
                json!({
                    "type": "string",
                    "description": "Nur nach einer Antwort „pending“: die approval_id daraus, sobald der Nutzer in der App freigegeben hat; dann denselben Aufruf mit denselben Argumenten wiederholen."
                }),
            );
        } else if schema.get("type").and_then(Value::as_str) == Some("object") {
            schema["properties"] = json!({ APPROVAL_ARG: { "type": "string",
                "description": "Nur nach einer Antwort „pending“: die approval_id daraus, sobald der Nutzer in der App freigegeben hat." } });
        }
    }
    let mut tool = json!({
        "name": name,
        "description": t.get("description").cloned().unwrap_or(Value::Null),
        "inputSchema": schema,
    });
    if let Some(title) = t.get("title") {
        tool["title"] = title.clone();
    }
    if let Some(a) = t.get("annotations") {
        tool["annotations"] = a.clone();
    }
    Some(tool)
}

/// Liest die Antwort von `tools/call`.
pub fn parse_call_reply(v: &Value) -> Result<LinkReply, LinkError> {
    match v.get("status").and_then(Value::as_str) {
        Some("done") => Ok(LinkReply::Done(
            v.get("result").cloned().unwrap_or(Value::Null),
        )),
        Some("pending") => Ok(LinkReply::Pending {
            approval_id: v
                .get("approval_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            message: v
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Die App wartet auf die Freigabe des Nutzers.")
                .to_string(),
        }),
        _ => Err(LinkError::new(
            LinkErrorKind::Failed,
            "Unerwartete Antwort von Local Voice AI.",
        )),
    }
}

impl BridgeLink for PipeLink {
    fn tools(&self) -> Result<Vec<Value>, LinkError> {
        self.bounded(LIST_DEADLINE, |s| {
            let list = s
                .request("tools/list", json!({}))
                .map_err(|e| explain(&e))?;
            Ok(list
                .get("tools")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(to_mcp_tool).collect())
                .unwrap_or_default())
        })
    }

    fn call(
        &self,
        name: &str,
        args: &Value,
        approval_id: Option<&str>,
    ) -> Result<LinkReply, LinkError> {
        let (name, args) = (name.to_string(), args.clone());
        let approval = approval_id.map(str::to_string);
        self.bounded(CALL_DEADLINE, move |s| {
            let mut params = json!({ "name": name, "arguments": args });
            if let Some(a) = approval {
                params["approval_id"] = json!(a);
            }
            let reply = s.request("tools/call", params).map_err(|e| explain(&e))?;
            parse_call_reply(&reply)
        })
    }
}

#[cfg(test)]
mod tests;
