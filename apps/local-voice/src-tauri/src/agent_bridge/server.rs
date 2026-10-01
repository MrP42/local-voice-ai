//! Eine Verbindung der Agentenbruecke: Zeilen lesen, `bridge` fragen, Antworten schreiben
//! (A7). Unabhaengig vom Transport: jede Verbindung ist ein beliebiger
//! `AsyncRead + AsyncWrite` (die echte Pipe in `pipe.rs`, in Tests ein `duplex`).
//!
//! Ablauf je Verbindung: `hello` (mit Token) meldet an; ohne Token bleibt die Verbindung
//! anonym und kann nur `status` (Lebenszeichen). Eine Anfrage nach der anderen; DB-Arbeit und
//! Werkzeuge laufen auf Arbeitsthreads (`spawn_blocking`), nie auf dem Lese-Task.
//!
//! Wartezeit auf „fragen“: Antwortet der Nutzer in der App innerhalb von
//! `Config::approval_wait` (30 s), laeuft das Werkzeug und die Antwort kommt direkt
//! zurueck; sonst kommt `{"status":"pending","approval_id":...}` und der Agent fragt spaeter
//! mit `approval/status` nach. Waehrend der Wartezeit wird die Verbindung beobachtet: geht der
//! Agent weg (Verbindungsende) oder endet die App, wird NICHTS ausgefuehrt.
//!
//! Schutz gegen Missbrauch: hoechstens `Config::max_connections` gleichzeitige Verbindungen
//! (weitere werden mit `too_many_connections` abgewiesen), Anmeldefrist, Leerlauf-Fristen,
//! Zeilen bis 1 MiB, nach `max_auth_failures_per_connection` Fehlanmeldungen wird die
//! Verbindung geschlossen, jede Anfrage prueft Zugang und Grenzen neu.

use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{
    split, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf,
};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use super::bridge::{Bridge, BridgeError, CallStep, ClientCtx, Phase};
use super::protocol::{self, code, Incoming, LineRead};
use super::MAX_LINE_BYTES;

pub struct Server {
    bridge: Arc<Bridge>,
    slots: Arc<Semaphore>,
    shutdown: watch::Sender<bool>,
}

/// Zustand einer Verbindung.
struct Session {
    client: Option<ClientCtx>,
    auth_failures: u32,
    /// Es kam schon mindestens eine Anfrage (die Anmeldefrist gilt nur fuer die erste).
    seen_request: bool,
}

/// Was nach einer Anfrage mit der Verbindung geschieht.
enum Next {
    Continue,
    /// Verbindung schliessen (nach der Antwort, falls es eine gibt).
    Close,
}

impl Server {
    pub fn new(bridge: Arc<Bridge>) -> Arc<Self> {
        let max = bridge.config().max_connections;
        let (shutdown, _) = watch::channel(false);
        Arc::new(Self {
            bridge,
            slots: Arc::new(Semaphore::new(max)),
            shutdown,
        })
    }

    pub fn bridge(&self) -> &Arc<Bridge> {
        &self.bridge
    }

    /// Beendet alle Verbindungen (sie melden `shutting_down`); neue werden abgewiesen.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
        self.slots.close();
    }

    pub fn shutdown_receiver(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// Freie Plaetze (Tests).
    pub fn free_slots(&self) -> usize {
        self.slots.available_permits()
    }

    /// Nimmt eine Verbindung an: ohne freien Platz wird sie mit `too_many_connections`
    /// abgewiesen, sonst bedient, bis sie endet.
    pub async fn handle<S>(self: Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        match self.slots.clone().try_acquire_owned() {
            Ok(permit) => self.serve(stream, permit).await,
            Err(_) => {
                let mut stream = stream;
                let closed = *self.shutdown.borrow();
                let (c, m) = if closed {
                    (code::SHUTTING_DOWN, "Local Voice AI wird beendet.")
                } else {
                    (
                        code::TOO_MANY_CONNECTIONS,
                        "Zu viele gleichzeitige Verbindungen. Bitte später erneut verbinden.",
                    )
                };
                let line = protocol::err_line(&Value::Null, c, m, None) + "\n";
                let _ = stream.write_all(line.as_bytes()).await;
                let _ = stream.flush().await;
                let _ = stream.shutdown().await;
            }
        }
    }

    async fn serve<S>(&self, stream: S, _permit: OwnedSemaphorePermit)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let cfg = self.bridge.config().clone();
        let (rd, mut wr) = split(stream);
        let mut rd = BufReader::new(rd);
        let mut shutdown = self.shutdown.subscribe();
        let mut session = Session {
            client: None,
            auth_failures: 0,
            seen_request: false,
        };
        loop {
            let limit = if session.client.is_some() {
                cfg.idle_timeout
            } else if session.seen_request {
                cfg.anonymous_idle_timeout
            } else {
                cfg.handshake_timeout
            };
            let read = tokio::select! {
                r = tokio::time::timeout(limit, protocol::read_line_capped(&mut rd, MAX_LINE_BYTES)) => match r {
                    Ok(Ok(read)) => read,
                    // Lesefehler oder Frist: die Verbindung endet still.
                    Ok(Err(_)) | Err(_) => break,
                },
                _ = shutdown.changed() => {
                    let _ = send(&mut wr, &protocol::err_line(&Value::Null, code::SHUTTING_DOWN, "Local Voice AI wird beendet.", None)).await;
                    break;
                }
            };
            let line = match read {
                LineRead::Eof => break,
                LineRead::TooLong => {
                    let _ = send(
                        &mut wr,
                        &protocol::err_line(
                            &Value::Null,
                            code::LINE_TOO_LONG,
                            "Die Zeile ist zu lang (höchstens 1 MiB).",
                            None,
                        ),
                    )
                    .await;
                    break;
                }
                LineRead::InvalidUtf8 => {
                    if send(
                        &mut wr,
                        &protocol::err_line(&Value::Null, code::BAD_REQUEST, "Die Zeile ist kein gültiges UTF-8.", None),
                    )
                    .await
                    .is_err()
                    {
                        break;
                    }
                    continue;
                }
                LineRead::Line(l) => l,
            };
            if line.trim().is_empty() {
                continue;
            }
            session.seen_request = true;
            let (reply, next) = match protocol::parse_request(&line) {
                Incoming::Invalid { id, message } => (
                    Some(protocol::err_line(&id, code::BAD_REQUEST, &message, None)),
                    Next::Continue,
                ),
                Incoming::Request { id, method, params } => {
                    self.dispatch(&mut session, &mut rd, &mut shutdown, &id, &method, &params).await
                }
            };
            if let Some(line) = reply {
                if send(&mut wr, &line).await.is_err() {
                    break;
                }
            }
            if matches!(next, Next::Close) {
                break;
            }
        }
        let _ = wr.shutdown().await;
    }

    async fn dispatch<S>(
        &self,
        session: &mut Session,
        rd: &mut BufReader<ReadHalf<S>>,
        shutdown: &mut watch::Receiver<bool>,
        id: &Value,
        method: &str,
        params: &Value,
    ) -> (Option<String>, Next)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let fail = |e: BridgeError| {
            (
                Some(protocol::err_line(id, e.code, &e.message, e.data.as_ref())),
                Next::Continue,
            )
        };
        let ok = |v: Value| (Some(protocol::ok_line(id, v)), Next::Continue);

        match method {
            "hello" => {
                let token = params.get("token").and_then(Value::as_str);
                let Some(token) = token else {
                    session.client = None;
                    let mut v = self.bridge.anonymous_status();
                    v["note"] = json!("Ohne Token ist nur status möglich.");
                    return ok(v);
                };
                let bridge = self.bridge.clone();
                let token = token.to_string();
                match blocking(move || bridge.authenticate(&token)).await {
                    Ok(ctx) => {
                        session.auth_failures = 0;
                        let mut v = self.bridge.anonymous_status();
                        v["authenticated"] = json!(true);
                        v["client"] = json!({ "id": ctx.id, "label": ctx.label });
                        session.client = Some(ctx);
                        ok(v)
                    }
                    Err(e) => {
                        session.client = None;
                        session.auth_failures += 1;
                        let close = session.auth_failures >= self.bridge.config().max_auth_failures_per_connection;
                        (
                            Some(protocol::err_line(id, e.code, &e.message, e.data.as_ref())),
                            if close { Next::Close } else { Next::Continue },
                        )
                    }
                }
            }
            "status" => match session.client.clone() {
                None => {
                    let bridge = self.bridge.clone();
                    match blocking(move || bridge.rate_check_anonymous()).await {
                        Ok(()) => ok(self.bridge.anonymous_status()),
                        Err(e) => fail(e),
                    }
                }
                Some(client) => {
                    let bridge = self.bridge.clone();
                    match blocking(move || {
                        let (ctx, _) = bridge.refresh(&client.id)?;
                        bridge.rate_check(&ctx, false)?;
                        bridge.status(&ctx)
                    })
                    .await
                    {
                        Ok(v) => ok(v),
                        Err(e) => {
                            let drop_session = matches!(e.code, code::TOKEN_REVOKED | code::TOKEN_INVALID);
                            if drop_session {
                                session.client = None;
                            }
                            fail(e)
                        }
                    }
                }
            },
            "tools/list" | "tools/call" | "approval/status" => {
                let Some(client) = session.client.clone() else {
                    return fail(BridgeError::new(
                        code::UNAUTHENTICATED,
                        "Nicht angemeldet: zuerst hello mit einem Token senden.",
                    ));
                };
                let is_call = method == "tools/call";
                let bridge = self.bridge.clone();
                let gate = {
                    let client = client.clone();
                    blocking(move || {
                        let (ctx, _) = bridge.refresh(&client.id)?;
                        bridge.rate_check(&ctx, is_call)?;
                        Ok(ctx)
                    })
                    .await
                };
                let ctx = match gate {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        if matches!(e.code, code::TOKEN_REVOKED | code::TOKEN_INVALID) {
                            session.client = None;
                        }
                        return fail(e);
                    }
                };
                match method {
                    "tools/list" => {
                        let bridge = self.bridge.clone();
                        match blocking(move || bridge.tools_list(&ctx)).await {
                            Ok(tools) => ok(json!({ "tools": tools })),
                            Err(e) => fail(e),
                        }
                    }
                    "approval/status" => {
                        let Some(approval_id) = params.get("approval_id").and_then(Value::as_str) else {
                            return fail(BridgeError::new(code::BAD_REQUEST, "approval_id fehlt."));
                        };
                        let approval_id = approval_id.to_string();
                        let bridge = self.bridge.clone();
                        match blocking(move || bridge.action_status(&ctx, &approval_id)).await {
                            Ok(v) => ok(v),
                            Err(e) => fail(e),
                        }
                    }
                    _ => self.tools_call(rd, shutdown, id, &ctx, params).await,
                }
            }
            other => fail(BridgeError::new(
                code::UNKNOWN_METHOD,
                format!("Die Methode „{}“ gibt es nicht.", other.chars().take(64).collect::<String>()),
            )),
        }
    }

    async fn tools_call<S>(
        &self,
        rd: &mut BufReader<ReadHalf<S>>,
        shutdown: &mut watch::Receiver<bool>,
        id: &Value,
        ctx: &ClientCtx,
        params: &Value,
    ) -> (Option<String>, Next)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let fail = |e: BridgeError| {
            (
                Some(protocol::err_line(id, e.code, &e.message, e.data.as_ref())),
                Next::Continue,
            )
        };
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return fail(BridgeError::new(code::BAD_REQUEST, "name fehlt oder ist kein Text."));
        };
        let args = params.get("arguments").cloned().unwrap_or(Value::Null);
        let approval_id = match params.get("approval_id") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => {
                return fail(BridgeError::new(code::BAD_REQUEST, "approval_id muss ein Text sein."));
            }
        };
        let (name, ctx2) = (name.to_string(), ctx.clone());
        let bridge = self.bridge.clone();
        let (n, a, ap) = (name.clone(), args.clone(), approval_id.clone());
        let step = blocking(move || bridge.call(&ctx2, &n, &a, ap.as_deref())).await;
        let approval = match step {
            Err(e) => return fail(e),
            Ok(CallStep::Done(v)) => {
                return (
                    Some(protocol::ok_line(id, json!({ "status": "done", "result": v }))),
                    Next::Continue,
                )
            }
            Ok(CallStep::Wait(approval)) => approval,
        };

        // Auf die Entscheidung des Nutzers warten (hoechstens `approval_wait`).
        match self.wait_for_decision(rd, shutdown, ctx, &approval).await {
            Wait::Cancelled => (None, Next::Close),
            Wait::Failed(e) => fail(e),
            Wait::TimedOut => (
                Some(protocol::ok_line(
                    id,
                    json!({
                        "status": "pending",
                        "approval_id": approval,
                        "message": "Die App wartet auf die Freigabe des Nutzers. Den Stand später mit approval/status (oder dem Werkzeug get_action_status) abfragen; nach „approved“ denselben Aufruf mit approval_id wiederholen.",
                    }),
                )),
                Next::Continue,
            ),
            Wait::Decided(Phase::Approved) => {
                let (bridge, ctx2, approval2) = (self.bridge.clone(), ctx.clone(), approval.clone());
                match blocking(move || bridge.call_approved(&ctx2, &name, &args, &approval2)).await {
                    Ok(v) => (
                        Some(protocol::ok_line(id, json!({ "status": "done", "result": v }))),
                        Next::Continue,
                    ),
                    Err(e) => fail(e),
                }
            }
            Wait::Decided(Phase::Denied) => fail(BridgeError::new(
                code::APPROVAL_DENIED,
                "Der Nutzer hat die Freigabe abgelehnt.",
            )),
            Wait::Decided(Phase::Expired) => fail(BridgeError::new(
                code::APPROVAL_EXPIRED,
                "Die Freigabe ist abgelaufen. Das Werkzeug bitte neu aufrufen.",
            )),
            Wait::Decided(_) => fail(BridgeError::new(
                code::APPROVAL_USED,
                "Diese Freigabe wurde schon verwendet.",
            )),
        }
    }

    /// Wartet auf `Approved`/`Denied`/`Expired` der Freigabe, hoechstens `approval_wait`.
    /// Endet die Verbindung des Agenten oder die App dabei, ist das Ergebnis `Cancelled`.
    async fn wait_for_decision<S>(
        &self,
        rd: &mut BufReader<ReadHalf<S>>,
        shutdown: &mut watch::Receiver<bool>,
        ctx: &ClientCtx,
        approval_id: &str,
    ) -> Wait
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let cfg = self.bridge.config();
        let deadline = Instant::now() + cfg.approval_wait;
        loop {
            let (bridge, ctx2, id2) = (self.bridge.clone(), ctx.clone(), approval_id.to_string());
            match blocking(move || bridge.approval_phase(&ctx2, &id2)).await {
                Err(e) => return Wait::Failed(e),
                Ok(info) if info.phase != Phase::Pending => return Wait::Decided(info.phase),
                Ok(_) => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return Wait::TimedOut;
            }
            let pause = cfg.poll_interval.min(deadline - now);
            tokio::select! {
                _ = tokio::time::sleep(pause) => {}
                _ = shutdown.changed() => return Wait::Cancelled,
                peek = rd.fill_buf() => match peek {
                    // Verbindung des Agenten zu: nichts ausfuehren.
                    Ok([]) | Err(_) => return Wait::Cancelled,
                    // Der Agent hat die naechste Anfrage vorausgeschickt: sie bleibt im Puffer
                    // und wird nach dieser Antwort gelesen; hier nur nicht im Kreis laufen.
                    Ok(_) => tokio::time::sleep(pause).await,
                },
            }
        }
    }
}

enum Wait {
    Decided(Phase),
    TimedOut,
    Cancelled,
    Failed(BridgeError),
}

async fn send<W: AsyncWrite + Unpin>(wr: &mut W, line: &str) -> std::io::Result<()> {
    let mut out = String::with_capacity(line.len() + 1);
    out.push_str(line);
    out.push('\n');
    wr.write_all(out.as_bytes()).await?;
    wr.flush().await
}

/// Fuehrt Datenbank- und Werkzeugarbeit auf einem Arbeitsthread aus; ein Absturz dort wird
/// zu einem Fehler statt die Verbindung zu reissen.
async fn blocking<T, F>(f: F) -> Result<T, BridgeError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, BridgeError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => r,
        Err(e) => {
            log::error!("agent_bridge: Arbeitsthread abgebrochen: {e}");
            Err(BridgeError::new(
                code::FAILED,
                "Interner Fehler. Es wurde nichts ausgeführt, was nicht im Protokoll steht.",
            ))
        }
    }
}

#[cfg(test)]
mod tests;
