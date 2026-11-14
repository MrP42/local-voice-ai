//! Abo-Relay fuer das Buch-Gespraech (lva#320, Spec
//! `docs/superpowers/specs/2026-11-05-buch-gespraech.md` 11.3, 11.4, 11.7).
//!
//! Das iPhone baut die Anfrage (Auszuege, Prompt) selbst und schickt sie ueber
//! den Ende-zu-Ende-verschluesselten Hub an diesen PC; der PC beantwortet sie
//! mit den eingeschalteten Claude-/Codex-Abo-Modellen (CLI, isoliert, ohne
//! Werkzeuge) und schreibt den Text fortlaufend zurueck. Spoilergrenze, Belege
//! und Prompt-Bau bleiben im iPhone-Kern -- der PC gibt Rohtext zurueck.
//!
//! # Hub-Objekte (Sammlung `booktalk_relay`)
//!
//! Siegel wie die uebrigen Objekte (`crypto::seal`, AAD = Benutzer, Sammlung,
//! Objekt-Kennung). Der Hub erlaubt in Objekt-Kennungen kein `/`
//! (`OBJECT_ID_RE` im wai-portal); die Spec schreibt `models/<id>`. Hier steht
//! `_`, beim Lesen gelten beide Trenner.
//!
//! * `models_<desktop_device_id>_<geraet>` -- je freigegebenem iPhone eine eigene,
//!   mit dessen Relay-Schluessel signierte Liste, siehe [`ModelsPayload`]
//!   (`<geraet>` = [`device_key`] des iPhones). Das fruehere gemeinsame Objekt
//!   `models_<desktop_device_id>` wird nicht mehr geschrieben und beim Start geloescht.
//! * `req_<request_id>` -- siehe [`ReqPayload`]
//! * `resp_<request_id>` -- siehe [`RespPayload`]
//!
//! # Zusaetzliche Authentifizierung (R1-R3)
//!
//! Konto-Schluessel und Hub-Zugang allein genuegen nie. Je freigegebenem iPhone
//! gibt es einen Relay-Schluessel (32 Byte, zufaellig); die Uebergabe laeuft
//! als Einrichtungscode am PC-Bildschirm ([`pairing_code`]). Zeiten sind
//! Unix-Sekunden (ganze Zahlen). Gemeinsame Golden-Datei mit dem iPhone-Client:
//! `tooling/fixtures/buch-gespraech/relay/golden.json`.
//!
//! MAC = `hex(HMAC-SHA-256(schluessel, kanonische Zeichenfolge als UTF-8))`. Die
//! Zeichenfolge ist JSON mit alphabetisch sortierten Schluesseln, ohne
//! Leerraum; `system` und `prompt` gehen nur als SHA-256 (hex) ein.
//!
//! * Anfrage: `created_at, device_id, expires_at, function, key_id, max_tokens,
//!   model_id, prompt_sha256, request_id, system_sha256, v`
//! * Antwort: `device_id, done, error ("" ohne), model_tuple, request_id, seq,
//!   text_sha256, v`
//! * Modellliste: `desktop_device_id, device_id, issued_at (Unix-Sekunden),
//!   models (nach `id` bytegenau aufsteigend sortiert, je Eintrag mit den
//!   Schluesseln context_tokens, id, label, provider), v`; `mac` liegt ausserhalb
//!
//! `usage` ist nicht Teil des MAC.
//!
//! Zeichenketten werden von [`json_quote`] maskiert: nur `"` `\\` `\n` `\r` `\t`
//! mit Kuerzel, alle anderen Zeichen unter 0x20 als `\u00xx` (Kleinbuchstaben),
//! alles Uebrige roh als UTF-8 (auch U+2028 und Emoji).
//!
//! # Antworten (R3) und Fehlercodes
//!
//! Beim Annehmen schreibt der PC sofort `seq 1` mit leerem Text und
//! `model_tuple "pending"` (Lebenszeichen vor der 15-s-Frist des iPhones).
//! Abweisungen tragen `model_tuple "unknown"`, `done:true` und `error`:
//! `not_allowed` (Funktion/Modell nicht freigegeben), `limit` (Groesse,
//! `max_tokens`, Stunden-/Tagesgrenze, schon eine Anfrage offen),
//! `model_unavailable`, `clock` (Uhrzeit passt nicht, siehe [`CLOCK_SKEW_S`]),
//! `timeout`, `llm_failed`.
//!
//! # Einrichtungscode und Freigabe (R1)
//!
//! Der Code gilt 10 Minuten und genau einmal: die Freigabe ist `pending`, bis
//! die erste gueltige (authentische) Anfrage des iPhones eintrifft; danach ist
//! sie aktiv, der Code verbraucht. Ohne diese erste Anfrage verfaellt die
//! Freigabe samt Schluessel nach 10 Minuten. Erneutes Anzeigen erzeugt einen
//! neuen Schluessel; der alte ist sofort ungueltig.
//!
//! Datenschutz (R5): nichts aus `system`, `prompt` oder der Antwort geht ins
//! Log; das Log kennt nur Zaehler und Fehlercodes.

use super::account::{self, SyncConfig};

/// Abo-Relay (iPhone -> PC-CLI): nur fuer gekoppelte Geraete DESSELBEN Kontos.
/// Erzwungen durch (1) Geraeteliste des Hubs desselben Benutzers (`device_listed`),
/// (2) je iPhone ein eigener Relay-Schluessel mit MAC (Einrichtungscode am
/// PC-Bildschirm) und (3) Siegel mit dem Benutzer als AAD. Die Anfragen gehen an
/// die eigene, unveraenderte CLI desselben Nutzers; ein Weiterleiten fuer fremde
/// Nutzer gibt es nicht und waere ohne Klaerung mit Anthropic nicht zulaessig
/// (docs/buch-gespraech/abo-vorschau-lizenz.md).
pub const SUBSCRIPTION_RELAY_SAME_ACCOUNT_ONLY: bool = true;
use super::client::{HubClient, HubDevice, HubError, PushObject, RemoteObject};
use super::crypto;
use crate::managers::llm::cli::Cli;
use crate::settings::AppSettings;
use base64::{
    engine::general_purpose::STANDARD as B64, engine::general_purpose::URL_SAFE_NO_PAD, Engine as _,
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::sync::Notify;

/// Sammlung im Hub.
pub const COLLECTION: &str = "booktalk_relay";
/// Version der Nutzlasten (`v`).
pub const PAYLOAD_VERSION: u32 = 1;
/// Die einzige erlaubte Funktion.
pub const FUNCTION: &str = "book_talk";
/// Hoechstgroesse von `system` + `prompt` in Zeichen.
pub const MAX_PROMPT_CHARS: usize = 120_000;
/// `system` geht als Argument an die CLI; Windows begrenzt die Zeile auf ~32.000 Zeichen.
pub const MAX_SYSTEM_CHARS: usize = 8_000;
/// Grenzen je Geraet (R6).
pub const MAX_TOKENS_LIMIT: u32 = 800;
pub const MAX_PER_HOUR: usize = 30;
pub const MAX_PER_DAY: usize = 200;
/// Fristen (R11), in Sekunden: `expires_at = created_at + 90`; der PC nimmt nur
/// an, wenn noch mindestens 65 bleiben.
///
/// Uhrabweichung: `created_at` darf von der PC-Uhr um hoechstens 120 s
/// abweichen, und zwar **in beide Richtungen** (iPhone-Uhr geht vor oder nach).
/// Zusaetzlich verlangt die 65-s-Regel, dass die Anfrage hoechstens 25 s nach
/// `created_at` (nach PC-Uhr) eintrifft; wer spaeter oder mit mehr als 25 s
/// nachgehender Uhr sendet, bekommt ebenfalls `clock`. Eine authentische, aber
/// zeitlich unpassende Anfrage wird mit `error:"clock"` beantwortet (nie still
/// verworfen), damit das iPhone „Uhr pruefen“ zeigen kann.
pub const REQUEST_WINDOW_S: i64 = 90;
pub const MIN_REMAINING_S: i64 = 65;
pub const CLOCK_SKEW_S: i64 = 120;
/// Einrichtungscode: gueltig 10 Minuten.
pub const CODE_TTL_MS: i64 = 10 * 60 * 1000;
/// Zeitrahmen eines Modellaufrufs (R11).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);
/// Abfragetakt des Hubs, solange das Relay angeboten wird.
pub const FAST_TICK: Duration = Duration::from_millis(1500);
/// Takt, solange das Relay aus ist (nur die Einstellung wird beobachtet).
const IDLE_TICK: Duration = Duration::from_secs(5);
/// Wartezeit nach einem Hub-Fehler.
const ERROR_TICK: Duration = Duration::from_secs(5);
/// So oft schreibt der PC den bisherigen Text zurueck.
const RESP_INTERVAL: Duration = Duration::from_secs(1);
/// So lange bleibt `resp` stehen, falls das iPhone es nicht loescht (R5).
pub const RESP_KEEP_MS: i64 = 5 * 60 * 1000;
/// Gesehene Anfragen bleiben bis Ablauf + 10 min gesperrt (R2), angenommene
/// 24 h (Zaehler fuer die Tagesgrenze).
const SEEN_GRACE_MS: i64 = 10 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Freigabe laeuft nach 90 Tagen ab (R1).
pub const GRANT_DAYS: i64 = 90;
/// Wie lange die Geraeteliste des Hubs gilt, bevor ein Unbekannter sie auffrischt.
const DEVICES_REFRESH: Duration = Duration::from_secs(3);
/// So lange gilt das Ergebnis der CLI-Pruefung (Anmeldung, Schalter).
const READY_TTL: Duration = Duration::from_secs(5 * 60);
/// Kontextgroesse, wenn das Modell keine eigene Angabe hat.
const DEFAULT_CONTEXT_TOKENS: u32 = 128_000;
/// Laenge einer Kennung (`req_` + Kennung bleibt unter den 64 Zeichen des Hubs).
const MAX_ID_LEN: usize = 48;
/// Der Hub traegt 256 KiB Klartext je Objekt; darunter bleiben.
const MAX_RESP_BYTES: usize = 180_000;
/// Modell-Angabe in Zwischenstaenden (das tatsaechliche kennt erst das Ende).
pub const MODEL_PENDING: &str = "pending";
/// Modell-Angabe, wenn die CLI das benutzte Modell nicht nennt.
pub const MODEL_UNKNOWN: &str = "unknown";

pub const ERR_MODEL_UNAVAILABLE: &str = "model_unavailable";
pub const ERR_TIMEOUT: &str = "timeout";
pub const ERR_LLM_FAILED: &str = "llm_failed";
pub const ERR_NOT_ALLOWED: &str = "not_allowed";
pub const ERR_LIMIT: &str = "limit";
pub const ERR_CLOCK: &str = "clock";

// ------------------------------------------------------------- Nutzlasten --

/// Ein Eintrag der veroeffentlichten Modellliste.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ModelEntry {
    /// Die Modell-Kennung der CLI (`sonnet`, `gpt-6.1-sol`); das tatsaechliche
    /// Modell der Antwort enthaelt sie (`claude-sonnet-5-5`), so prueft das iPhone.
    pub id: String,
    /// Kennung des Modells in den Einstellungen dieses PCs (nicht im Hub).
    #[serde(skip)]
    pub config_id: String,
    pub label: String,
    /// `claude_cli` oder `codex_cli`.
    pub provider: String,
    pub context_tokens: u32,
}

/// `models_<desktop_device_id>_<geraet>`: die Modelle, die dieses iPhone nutzen
/// darf, mit HMAC ueber [`models_canonical`] (Schluessel dieses Geraets).
/// Entspricht `BookTalkRelayModelList` im iPhone-Client.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ModelsPayload {
    pub v: u32,
    /// Geraete-Kennung dieses PCs (Absender).
    pub desktop_device_id: String,
    /// Geraete-Kennung des iPhones, fuer das die Liste gilt.
    pub device_id: String,
    /// Unix-Sekunden der Veroeffentlichung.
    pub issued_at: i64,
    pub models: Vec<ModelEntry>,
    /// Nicht Teil der Signatur.
    pub mac: String,
}

/// `req_<request_id>`: die fertig gebaute, signierte Anfrage des iPhones.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ReqPayload {
    pub v: u32,
    /// 128 Bit Zufall (Buchstaben, Ziffern, `-`, `_`).
    pub request_id: String,
    /// Geraete-Kennung des iPhones (wie im Hub).
    pub device_id: String,
    /// Kennung des Relay-Schluessels.
    pub key_id: String,
    /// Immer `book_talk`.
    pub function: String,
    pub model_id: String,
    #[serde(default)]
    pub system: String,
    pub prompt: String,
    pub max_tokens: u32,
    /// Unix-Sekunden.
    pub created_at: i64,
    /// `created_at + 90`.
    pub expires_at: i64,
    /// Siehe Moduldoku.
    pub mac: String,
}

/// Verbrauch einer Antwort (Token, soweit die CLI sie meldet).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// `resp_<request_id>`: der bisher erzeugte Text, `done` am Ende, mit MAC.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RespPayload {
    pub v: u32,
    pub request_id: String,
    /// Geraete-Kennung des anfragenden iPhones.
    pub device_id: String,
    pub seq: u32,
    /// Das tatsaechlich benutzte Modell laut CLI-Ausgabe ([`MODEL_PENDING`] in
    /// Zwischenstaenden, [`MODEL_UNKNOWN`] wenn die CLI es nicht nennt).
    pub model_tuple: String,
    pub text: String,
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Abo-Vorschau (lva#334): die Antwort stammt aus einem CLI-Abo mit nur
    /// empirischem Budget. Wie `usage` nicht Teil des MAC; aeltere Clients
    /// ignorieren das Feld (die Oberflaeche des iPhones kennzeichnet Relay-
    /// Antworten ohne bestandene Pruefung ohnehin als Vorschau).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
    pub mac: String,
}

// ------------------------------------------------------------------ MAC ----

type HmacSha256 = Hmac<Sha256>;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

pub fn mac_hex(key: &[u8], input: &[u8]) -> String {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC nimmt jede Schluessellaenge");
    m.update(input);
    hex(&m.finalize().into_bytes())
}

/// Konstantzeit-Pruefung.
pub fn mac_ok(key: &[u8], input: &[u8], mac_hex_value: &str) -> bool {
    let Some(expected) = unhex(mac_hex_value) else {
        return false;
    };
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC nimmt jede Schluessellaenge");
    m.update(input);
    m.verify_slice(&expected).is_ok()
}

/// JSON-Zeichenkette wie die Swift-Fassung (`BookTalkCanonical.quote`):
/// nur `"`, `\`, `\n`, `\r`, `\t` und Steuerzeichen werden maskiert.
pub fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Kanonische Zeichenfolge einer Anfrage (R2).
pub fn req_canonical(r: &ReqPayload) -> String {
    format!(
        "{{\"created_at\":{},\"device_id\":{},\"expires_at\":{},\"function\":{},\"key_id\":{},\"max_tokens\":{},\"model_id\":{},\"prompt_sha256\":{},\"request_id\":{},\"system_sha256\":{},\"v\":{}}}",
        r.created_at,
        json_quote(&r.device_id),
        r.expires_at,
        json_quote(&r.function),
        json_quote(&r.key_id),
        r.max_tokens,
        json_quote(&r.model_id),
        json_quote(&sha256_hex(&r.prompt)),
        json_quote(&r.request_id),
        json_quote(&sha256_hex(&r.system)),
        r.v
    )
}

/// Kanonische Zeichenfolge einer Antwort (R3).
pub fn resp_canonical(r: &RespPayload) -> String {
    format!(
        "{{\"device_id\":{},\"done\":{},\"error\":{},\"model_tuple\":{},\"request_id\":{},\"seq\":{},\"text_sha256\":{},\"v\":{}}}",
        json_quote(&r.device_id),
        r.done,
        json_quote(r.error.as_deref().unwrap_or("")),
        json_quote(&r.model_tuple),
        json_quote(&r.request_id),
        r.seq,
        json_quote(&sha256_hex(&r.text)),
        r.v
    )
}

/// Kanonische Zeichenfolge einer Modellliste (wie `BookTalkRelayModelList.canonical`):
/// Schluessel alphabetisch, `v` mitsigniert, `mac` nicht; die Eintraege nach
/// `id` (UTF-8-Bytes) sortiert, je `context_tokens, id, label, provider`.
pub fn models_canonical(p: &ModelsPayload) -> String {
    let mut sorted: Vec<&ModelEntry> = p.models.iter().collect();
    sorted.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
    let models: Vec<String> = sorted
        .iter()
        .map(|m| {
            format!(
                "{{\"context_tokens\":{},\"id\":{},\"label\":{},\"provider\":{}}}",
                m.context_tokens,
                json_quote(&m.id),
                json_quote(&m.label),
                json_quote(&m.provider)
            )
        })
        .collect();
    format!(
        "{{\"desktop_device_id\":{},\"device_id\":{},\"issued_at\":{},\"models\":[{}],\"v\":{}}}",
        json_quote(&p.desktop_device_id),
        json_quote(&p.device_id),
        p.issued_at,
        models.join(","),
        p.v
    )
}

/// Modellliste fuer ein Geraet bauen und signieren (`now_ms` -> `issued_at` in Sekunden).
pub fn signed_models(
    desktop_device_id: &str,
    grant: &DeviceGrant,
    key: &[u8],
    mut models: Vec<ModelEntry>,
    now_ms: i64,
) -> ModelsPayload {
    models.sort_by(|a, b| a.id.as_bytes().cmp(b.id.as_bytes()));
    let mut p = ModelsPayload {
        v: PAYLOAD_VERSION,
        desktop_device_id: desktop_device_id.to_string(),
        device_id: grant.device_id.clone(),
        issued_at: now_ms.div_euclid(1000),
        models,
        mac: String::new(),
    };
    p.mac = mac_hex(key, models_canonical(&p).as_bytes());
    p
}

/// Modellliste pruefen (so, wie das iPhone es tut; hier fuer die Tests).
pub fn models_mac_ok(key: &[u8], p: &ModelsPayload) -> bool {
    mac_ok(key, models_canonical(p).as_bytes(), &p.mac)
}

/// Antwort pruefen (so, wie das iPhone es tut; hier fuer die Tests).
pub fn resp_mac_ok(key: &[u8], r: &RespPayload) -> bool {
    mac_ok(key, resp_canonical(r).as_bytes(), &r.mac)
}

// ------------------------------------------------------- Objekt-Kennungen --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Models,
    Req,
    Resp,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::Models => "models",
            Kind::Req => "req",
            Kind::Resp => "resp",
        }
    }
}

pub fn object_id(kind: Kind, id: &str) -> String {
    format!("{}_{id}", kind.prefix())
}

/// Art und Kennung aus einer Objekt-Kennung; `_` und `/` gelten als Trenner.
pub fn parse_object_id(object_id: &str) -> Option<(Kind, &str)> {
    [Kind::Models, Kind::Req, Kind::Resp]
        .into_iter()
        .find_map(|k| {
            let rest = object_id.strip_prefix(k.prefix())?;
            let id = rest.strip_prefix('_').or_else(|| rest.strip_prefix('/'))?;
            (!id.is_empty()).then_some((k, id))
        })
}

/// Kennung einer Anfrage: Buchstaben, Ziffern, `-`, `_`, hoechstens 48 Zeichen.
pub fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Kurzkennung eines Geraets, wie der Hub sie in der Geraeteliste fuehrt
/// (`VoiceSyncService::deviceKey`: die ersten 12 Hex-Zeichen von SHA-256).
pub fn device_key(device_id: &str) -> String {
    hex(&Sha256::digest(device_id.as_bytes())[..6])
}

pub fn valid_device_key(k: &str) -> bool {
    k.len() == 12 && k.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// -------------------------------------------------- Freigaben und Ablage ----

/// Freigabe eines iPhones (R1): Schluessel, Funktion, Modelle, Ablauf.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DeviceGrant {
    /// Volle Geraete-Kennung des iPhones im Hub.
    pub device_id: String,
    /// Kurzkennung des Geraets im Hub (`device_key(device_id)`).
    pub device_key: String,
    pub name: String,
    pub key_id: String,
    /// Relay-Schluessel, base64. Liegt nur in dieser Datei (0600).
    pub key_b64: String,
    pub functions: Vec<String>,
    pub model_ids: Vec<String>,
    pub created_at_ms: i64,
    /// Ablauf der Freigabe (90 Tage), getrennt vom Ablauf des Codes.
    pub expires_at_ms: i64,
    /// `false`, solange noch keine gueltige Anfrage des iPhones eingetroffen ist.
    #[serde(default = "default_true")]
    pub activated: bool,
    /// Bis dahin gilt der Einrichtungscode (und damit eine noch nicht
    /// aktivierte Freigabe).
    #[serde(default)]
    pub code_expires_at_ms: i64,
}

fn default_true() -> bool {
    true
}

impl DeviceGrant {
    pub fn key(&self) -> Option<Vec<u8>> {
        B64.decode(&self.key_b64).ok().filter(|k| k.len() == 32)
    }

    /// Gueltig: nicht abgelaufen, und entweder aktiviert oder der Code gilt noch.
    pub fn active(&self, now_ms: i64) -> bool {
        self.expires_at_ms > now_ms && (self.activated || self.code_expires_at_ms > now_ms)
    }

    /// Noch nicht benutzt, Code noch gueltig.
    pub fn pending(&self, now_ms: i64) -> bool {
        !self.activated && self.code_expires_at_ms > now_ms
    }
}

pub fn key_id_of(key: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(b"lv-booktalk-relay-key-id-v1");
    h.update(key);
    hex(&h.finalize()[..8])
}

/// Eine bearbeitete Anfrage (Replay-Sperre und Zaehler, R2/R6).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Seen {
    pub request_id: String,
    pub device_key: String,
    /// Zeitpunkt der Annahme; 0 bei abgewiesenen.
    pub accepted_ms: i64,
    /// Bis dahin bleibt die Kennung gesperrt.
    pub until_ms: i64,
}

/// Vorgemerktes Loeschen eines Hub-Objekts (ueberlebt einen Neustart, R5).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PendingCleanup {
    pub object_id: String,
    pub due_ms: i64,
}

/// Dauerhafter Zustand auf Platte: `booktalk_relay.json`.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
pub struct RelayStore {
    #[serde(default)]
    pub grants: Vec<DeviceGrant>,
    #[serde(default)]
    pub seen: Vec<Seen>,
    #[serde(default)]
    pub cleanup: Vec<PendingCleanup>,
}

impl RelayStore {
    /// Die gueltige Freigabe fuer Geraet und Schluessel-ID.
    pub fn grant_for(&self, device_id: &str, key_id: &str, now_ms: i64) -> Option<&DeviceGrant> {
        self.grants
            .iter()
            .find(|g| g.device_id == device_id && g.key_id == key_id && g.active(now_ms))
    }

    pub fn has_seen(&self, request_id: &str) -> bool {
        self.seen.iter().any(|s| s.request_id == request_id)
    }

    /// Anfrage merken; `accepted` zaehlt gegen die Stunden- und Tagesgrenze.
    pub fn record(
        &mut self,
        request_id: &str,
        device_key: &str,
        accepted: bool,
        expires_ms: i64,
        now_ms: i64,
    ) {
        // Auch bei einer Anfrage mit abwegiger Uhrzeit mindestens `SEEN_GRACE_MS` ab jetzt sperren.
        let until = if accepted {
            (expires_ms + SEEN_GRACE_MS).max(now_ms + DAY_MS)
        } else {
            expires_ms.max(now_ms) + SEEN_GRACE_MS
        };
        self.seen.push(Seen {
            request_id: request_id.to_string(),
            device_key: device_key.to_string(),
            accepted_ms: if accepted { now_ms } else { 0 },
            until_ms: until,
        });
    }

    pub fn prune(&mut self, now_ms: i64) {
        self.seen.retain(|s| s.until_ms > now_ms);
        // Nie benutzter Einrichtungscode abgelaufen: Freigabe samt Schluessel weg.
        self.grants
            .retain(|g| g.activated || g.code_expires_at_ms > now_ms);
    }

    /// Erste gueltige Anfrage des iPhones: Freigabe aktivieren (der Code ist damit
    /// verbraucht). `true`, wenn sie vorher noch `pending` war.
    pub fn activate(&mut self, device_id: &str, key_id: &str) -> bool {
        match self
            .grants
            .iter_mut()
            .find(|g| g.device_id == device_id && g.key_id == key_id && !g.activated)
        {
            Some(g) => {
                g.activated = true;
                true
            }
            None => false,
        }
    }

    /// Angenommene Anfragen eines Geraets seit `since_ms`.
    pub fn accepted_since(&self, device_key: &str, since_ms: i64) -> usize {
        self.seen
            .iter()
            .filter(|s| {
                s.device_key == device_key && s.accepted_ms > 0 && s.accepted_ms >= since_ms
            })
            .count()
    }

    pub fn add_grant(&mut self, grant: DeviceGrant) {
        self.grants.retain(|g| g.device_key != grant.device_key);
        self.grants.push(grant);
    }

    /// Freigabe entziehen (Schluessel loeschen). `true`, wenn es eine gab.
    pub fn revoke(&mut self, device_key: &str) -> bool {
        let before = self.grants.len();
        self.grants.retain(|g| g.device_key != device_key);
        before != self.grants.len()
    }
}

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|d| d.join("booktalk_relay.json"))
        .map_err(|e| format!("Kein Datenverzeichnis: {e}"))
}

static STORE_LOCK: Mutex<()> = Mutex::new(());

/// Ergebnis des Lesens der Ablage.
enum Loaded {
    Missing,
    Ok(RelayStore),
    /// Inhalt nicht lesbar als `RelayStore`.
    Corrupt,
    /// Datei da, aber nicht zu lesen (Rechte, Sperre): nichts anfassen.
    Unreadable,
}

fn read_store(path: &Path) -> Loaded {
    match std::fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str(&raw) {
            Ok(store) => Loaded::Ok(store),
            Err(_) => Loaded::Corrupt,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Loaded::Missing,
        Err(_) => Loaded::Unreadable,
    }
}

/// Lesen unter der Sperre (auch fuer den Widerrufs-Test laufender Anfragen).
/// Eine fehlende oder unlesbare Ablage gilt als leer; geschrieben wird hier nie.
pub fn load_store(path: &Path) -> RelayStore {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    match read_store(path) {
        Loaded::Ok(store) => store,
        _ => RelayStore::default(),
    }
}

/// Laden, aendern, schreiben -- unter einer Sperre, weil Oberflaeche und
/// Schleife dieselbe Datei benutzen. Geschrieben wird nur bei echter
/// Aenderung, atomar (Nachbardatei + Umbenennen). Eine nicht lesbare Datei
/// wird nie mit einem Standardwert ueberschrieben: ein Parse-Fehler legt sie
/// beiseite (`.defekt-<zeit>`), das Relay bleibt aus (keine Freigaben), bis
/// neu freigegeben wird; eine unlesbare Datei ergibt einen Fehler.
pub fn update_store<R>(path: &Path, f: impl FnOnce(&mut RelayStore) -> R) -> Result<R, String> {
    let _guard = STORE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut store = match read_store(path) {
        Loaded::Ok(store) => store,
        Loaded::Missing => RelayStore::default(),
        Loaded::Corrupt => {
            let mut name = path
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(format!(".defekt-{}", now_ms()));
            std::fs::rename(path, path.with_file_name(name))
                .map_err(|e| format!("Relay-Ablage defekt und nicht beiseite zu legen: {e}"))?;
            log::warn!("Buch-Relay: Ablage unlesbar, beiseite gelegt; das Relay bleibt aus, bis neu freigegeben wird");
            RelayStore::default()
        }
        Loaded::Unreadable => return Err("Relay-Ablage nicht lesbar".into()),
    };
    let before = store.clone();
    let out = f(&mut store);
    if store == before {
        return Ok(out);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Datenverzeichnis: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    account::write_private(path, raw.as_bytes())?;
    Ok(out)
}

/// Einrichtungscode fuer das iPhone: `lvrelay1:` plus JSON `{v, key_id, k
/// (base64url, 32 Byte), desktop_device_id, desktop_name, device_id, model_ids,
/// expires_at, grant_expires_at}` -- wie `BookTalkRelayEnrollment` im iPhone-Client.
/// `expires_at` = Ablauf des **Codes** (10 Minuten, Unix-Sekunden),
/// `grant_expires_at` = Ablauf der **Freigabe** (90 Tage, Unix-Sekunden).
pub fn pairing_code(desktop_device_id: &str, desktop_name: &str, grant: &DeviceGrant) -> String {
    let key = B64.decode(&grant.key_b64).unwrap_or_default();
    let body = serde_json::json!({
        "v": 1,
        "key_id": grant.key_id,
        "k": URL_SAFE_NO_PAD.encode(key),
        "desktop_device_id": desktop_device_id,
        "desktop_name": desktop_name,
        "device_id": grant.device_id,
        "model_ids": grant.model_ids,
        "expires_at": grant.code_expires_at_ms.div_euclid(1000),
        "grant_expires_at": grant.expires_at_ms.div_euclid(1000),
    });
    format!("lvrelay1:{body}")
}

/// Neue Freigabe: frischer Schluessel, `pending` bis zur ersten gueltigen
/// Anfrage, Code 10 Minuten, Freigabe 90 Tage.
pub fn new_grant(device_id: &str, name: &str, model_ids: Vec<String>, now_ms: i64) -> DeviceGrant {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    DeviceGrant {
        device_id: device_id.to_string(),
        device_key: device_key(device_id),
        name: name.chars().take(60).collect(),
        key_id: key_id_of(&key),
        key_b64: B64.encode(key),
        functions: vec![FUNCTION.to_string()],
        model_ids,
        created_at_ms: now_ms,
        expires_at_ms: now_ms + GRANT_DAYS * DAY_MS,
        activated: false,
        code_expires_at_ms: now_ms + CODE_TTL_MS,
    }
}

/// QR-Code als SVG (Bibliothek `qrcode`, MIT OR Apache-2.0): ein Pfad aus
/// Einzelmodulen, mit Ruhezone, ohne Skripte; die Oberflaeche zeigt ihn als Bild.
pub fn qr_svg(text: &str) -> Result<String, String> {
    use qrcode::{Color, EcLevel, QrCode};
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|e| format!("QR-Code: {e}"))?;
    let width = code.width();
    let quiet = 4;
    let size = width + 2 * quiet;
    let mut path = String::new();
    for (i, c) in code.to_colors().into_iter().enumerate() {
        if c == Color::Dark {
            path.push_str(&format!(
                "M{} {}h1v1h-1z",
                i % width + quiet,
                i / width + quiet
            ));
        }
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {size} {size}\" shape-rendering=\"crispEdges\"><rect width=\"{size}\" height=\"{size}\" fill=\"#fff\"/><path d=\"{path}\" fill=\"#000\"/></svg>"
    ))
}

// ------------------------------------------------------------ Modellliste --

/// Welche CLIs sind bereit (angemeldet per Abo, kein API-Schluessel, alle
/// Isolationsschalter vorhanden)? Siehe [`crate::managers::llm::cli::relay_ready`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ready {
    pub claude: bool,
    pub codex: bool,
}

impl Ready {
    pub fn of(self, cli: Cli) -> bool {
        match cli {
            Cli::Claude => self.claude,
            Cli::Codex => self.codex,
        }
    }
}

/// Die Modelle, die dieser PC prinzipiell anbieten kann: freigegebene,
/// eingeschaltete Modelle eingeschalteter Abo-Verbindungen, deren CLI bereit
/// ist (R4, R6). Leer, solange der Schalter `book_talk_relay_enabled` aus ist.
pub fn eligible_models(settings: &AppSettings, ready: Ready) -> Vec<ModelEntry> {
    if !settings.book_talk_relay_enabled {
        return Vec::new();
    }
    settings
        .llm_models
        .iter()
        .filter(|m| m.enabled)
        .filter_map(|m| {
            let connection = settings
                .llm_connections
                .iter()
                .find(|c| c.id == m.connection_id && c.enabled)?;
            let cli = Cli::from_base_url(&connection.base_url)?;
            if !ready.of(cli) {
                return None;
            }
            let provider = match cli {
                Cli::Claude => "claude_cli",
                Cli::Codex => "codex_cli",
            };
            let label = if m.label.trim().is_empty() {
                m.remote_id.clone()
            } else {
                m.label.clone()
            };
            Some(ModelEntry {
                id: m.remote_id.clone(),
                config_id: m.id.clone(),
                label,
                provider: provider.to_string(),
                context_tokens: m.context_limit.unwrap_or(DEFAULT_CONTEXT_TOKENS),
            })
        })
        .fold(Vec::<ModelEntry>::new(), |mut acc, m| {
            // Zwei Modelle mit gleicher CLI-Kennung waeren fuer das iPhone nicht zu unterscheiden.
            if !acc.iter().any(|e| e.id == m.id) {
                acc.push(m);
            }
            acc
        })
}

/// Programm-Identitaet (Version, Pruefsumme) je CLI, fuer die Bindung der
/// Abo-Vorschau (lva#334). `None`: CLI nicht installiert oder nicht lesbar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Identities {
    pub claude: Option<crate::managers::llm::preview::Identity>,
}

impl Identities {
    pub fn of(&self, cli: Cli) -> Option<&crate::managers::llm::preview::Identity> {
        match cli {
            Cli::Claude => self.claude.as_ref(),
            // Phase 1: Codex gibt es in der Vorschau nicht.
            Cli::Codex => None,
        }
    }
}

/// Die Modelle, die wirklich angeboten werden duerfen (lva#334): nur mit
/// eingeschalteter Abo-Vorschau und `Active` im Tor: gueltiges gebundenes
/// Tupel (Anbieter, Modell, CLI-Version, SHA-256 des Programms), alle
/// `IsolationCaps` der Version belegt, gueltige Messung, belegter Kontext.
/// `context_tokens` ist der belegte Wert (Claude 200000, Codex nur ein
/// eingetragener), nie ein pauschaler Standard.
pub fn preview_models(
    settings: &AppSettings,
    eligible: Vec<ModelEntry>,
    identities: &Identities,
    evidence: &crate::managers::llm::evidence::Evidence,
) -> Vec<ModelEntry> {
    use crate::managers::llm::preview;
    if !settings.book_talk_subscription_preview {
        return Vec::new();
    }
    eligible
        .into_iter()
        .filter_map(|mut m| {
            let cli = match m.provider.as_str() {
                "claude_cli" => Cli::Claude,
                _ => return None,
            };
            let binding = preview::binding_for(cli, &m.id, identities.of(cli)?);
            let gate = preview::gate(
                true,
                &settings.book_talk_preview_bindings,
                Some(&binding),
                evidence,
            );
            if gate.state != preview::PreviewState::Active {
                return None;
            }
            let configured = settings
                .llm_models
                .iter()
                .find(|c| c.id == m.config_id)
                .and_then(|c| c.context_limit);
            m.context_tokens = preview::context_tokens(cli, configured)?;
            Some(m)
        })
        .collect()
}

/// Wie [`eligible_models`], aber nur, was die Abo-Vorschau freigibt
/// ([`preview_models`]). Liest die Programm-Identitaet der bereiten CLIs.
async fn offered_models(
    settings: &AppSettings,
    evidence: &crate::managers::llm::evidence::Evidence,
) -> Vec<ModelEntry> {
    let eligible = eligible_models(settings, ready_now().await);
    if eligible.is_empty() || !settings.book_talk_subscription_preview {
        return Vec::new();
    }
    let identities = tokio::task::spawn_blocking(|| {
        let of = |cli: Cli| {
            crate::managers::llm::cli::locate_native_claude()
                .ok()
                .filter(|_| cli == Cli::Claude)
                .and_then(|binary| crate::managers::llm::preview::identity(&binary).ok())
        };
        Identities {
            claude: of(Cli::Claude),
        }
    })
    .await
    .unwrap_or_default();
    preview_models(settings, eligible, &identities, evidence)
}

/// Veroeffentlicht wird, was mindestens ein Geraet nutzen darf (R1). Ohne
/// Freigabe bleibt die Liste leer -- dann wird das Objekt geloescht.
pub fn published_models(
    eligible: &[ModelEntry],
    grants: &[DeviceGrant],
    now_ms: i64,
) -> Vec<ModelEntry> {
    eligible
        .iter()
        .filter(|m| {
            grants
                .iter()
                .any(|g| g.active(now_ms) && g.model_ids.iter().any(|id| id == &m.id))
        })
        .cloned()
        .collect()
}

/// Fingerabdruck einer Geraeteliste (ohne `updated_at`, mit Schluessel-ID, weil
/// ein neuer Schluessel eine neu signierte Liste braucht).
pub fn models_fingerprint(key_id: &str, models: &[ModelEntry]) -> String {
    sha256_hex(&format!(
        "{key_id}|{}",
        serde_json::to_string(models).unwrap_or_default()
    ))
}

/// Objekt-Kennung der Liste fuer ein Geraet: `models_<desktop>_<geraet>`.
pub fn list_object_id(desktop_device_id: &str, device_key: &str) -> String {
    object_id(Kind::Models, &format!("{desktop_device_id}_{device_key}"))
}

/// Die Liste je gueltiger Freigabe (auch einer noch nicht aktivierten, damit
/// das iPhone nach dem Einlesen des Codes die Modelle sieht): die
/// eingeschalteten, bereiten Modelle, die die Freigabe erlaubt. Geraete ohne
/// Modell bekommen keine Liste.
pub fn device_lists(
    eligible: &[ModelEntry],
    grants: &[DeviceGrant],
    now_ms: i64,
) -> BTreeMap<String, (DeviceGrant, Vec<ModelEntry>)> {
    grants
        .iter()
        .filter(|g| g.active(now_ms))
        .filter_map(|g| {
            let entries: Vec<ModelEntry> = eligible
                .iter()
                .filter(|m| g.model_ids.iter().any(|id| id == &m.id))
                .cloned()
                .collect();
            (!entries.is_empty()).then(|| (g.device_key.clone(), (g.clone(), entries)))
        })
        .collect()
}

/// Beim Start: das fruehere gemeinsame `models_<desktop>` und Listen von
/// Geraeten ohne gueltige Freigabe loeschen (Hub-Objekte haben keine TTL).
async fn sweep_lists(
    ctx: &Arc<Ctx>,
    st: &mut LoopState,
    keep: &BTreeSet<String>,
) -> Result<(), HubError> {
    let status = ctx.client.status().await?;
    st.devices = status.devices.clone();
    st.devices_at = Some(Instant::now());
    if status.objects.get(COLLECTION).copied().unwrap_or(0) == 0 {
        return Ok(());
    }
    ctx.put(&object_id(Kind::Models, &ctx.cfg.device_id), None)
        .await?;
    let own = device_key(&ctx.cfg.device_id);
    for d in &status.devices {
        if valid_device_key(&d.device) && d.device != own && !keep.contains(&d.device) {
            ctx.put(&list_object_id(&ctx.cfg.device_id, &d.device), None)
                .await?;
        }
    }
    Ok(())
}

/// Einstellungen so, dass das Modell `model_id` das aktive ist (mit Effort,
/// Fast-Endung und Verbindung), wie `for_function` es fuer eine Funktion tut.
fn settings_for_model(settings: &AppSettings, model_id: &str) -> AppSettings {
    let mut s = settings.clone();
    s.llm_active_model_id = Some(model_id.to_string());
    s.sync_legacy_from_llm();
    s
}

// ------------------------------------------------------------- Pruefregeln --

/// Zustand, gegen den eine Anfrage geprueft wird.
pub struct Gate<'a> {
    pub now_ms: i64,
    /// Geraete-Kennung, mit der das Objekt im Hub ankam.
    pub hub_device_id: &'a str,
    pub published: &'a [ModelEntry],
    /// Fuer dieses Geraet laeuft schon eine Anfrage.
    pub busy: bool,
    pub store: &'a RelayStore,
}

/// Ergebnis der Pruefung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Ausfuehren.
    Accept,
    /// Keine Antwort schreiben (ungueltig, nicht authentisch, abgelaufen, Replay).
    Ignore(&'static str),
    /// Authentische Anfrage, mit diesem Fehlercode beantworten.
    Reject(&'static str),
}

/// Prueft eine entschluesselte Anfrage in der Reihenfolge der Spec (11.7):
/// Freigabe und MAC zuerst, damit ein Unbefugter weder Antworten noch
/// Eintraege in der Replay-Sperre ausloest. Nur eine authentische Anfrage
/// (`Accept`/`Reject`) aktiviert eine noch `pending` Freigabe.
pub fn check_request(object_request_id: &str, req: &ReqPayload, gate: &Gate<'_>) -> Verdict {
    if req.v != PAYLOAD_VERSION {
        return Verdict::Ignore("version");
    }
    if !valid_request_id(&req.request_id)
        || req.request_id != object_request_id
        || req.device_id.is_empty()
        || req.device_id != gate.hub_device_id
    {
        return Verdict::Ignore("kennung");
    }
    let dkey = device_key(&req.device_id);
    let Some(grant) = gate
        .store
        .grant_for(&req.device_id, &req.key_id, gate.now_ms)
    else {
        return Verdict::Ignore("keine_freigabe");
    };
    let Some(key) = grant.key() else {
        return Verdict::Ignore("schluessel");
    };
    if !mac_ok(&key, req_canonical(req).as_bytes(), &req.mac) {
        return Verdict::Ignore("mac");
    }
    // Ab hier ist die Anfrage authentisch (MAC stimmt): zeitlich Unpassendes
    // wird mit `clock` beantwortet, nicht still verworfen (Wiederholungen sperrt
    // die Replay-Sperre).
    if req.expires_at != req.created_at + REQUEST_WINDOW_S {
        return Verdict::Ignore("frist");
    }
    if gate.store.has_seen(&req.request_id) {
        return Verdict::Ignore("replay");
    }
    let now_s = gate.now_ms.div_euclid(1000);
    if (req.created_at - now_s).abs() > CLOCK_SKEW_S || req.expires_at - now_s < MIN_REMAINING_S {
        return Verdict::Reject(ERR_CLOCK);
    }
    if req.function != FUNCTION || !grant.functions.iter().any(|f| f == &req.function) {
        return Verdict::Reject(ERR_NOT_ALLOWED);
    }
    if !grant.model_ids.iter().any(|m| m == &req.model_id) {
        return Verdict::Reject(ERR_NOT_ALLOWED);
    }
    if !gate.published.iter().any(|m| m.id == req.model_id) {
        return Verdict::Reject(ERR_MODEL_UNAVAILABLE);
    }
    let (sys, prompt) = (req.system.chars().count(), req.prompt.chars().count());
    if sys > MAX_SYSTEM_CHARS || sys + prompt > MAX_PROMPT_CHARS {
        return Verdict::Reject(ERR_LIMIT);
    }
    if req.max_tokens == 0 || req.max_tokens > MAX_TOKENS_LIMIT {
        return Verdict::Reject(ERR_LIMIT);
    }
    if gate.store.accepted_since(&dkey, gate.now_ms - 3_600_000) >= MAX_PER_HOUR
        || gate.store.accepted_since(&dkey, gate.now_ms - DAY_MS) >= MAX_PER_DAY
    {
        return Verdict::Reject(ERR_LIMIT);
    }
    if gate.busy {
        return Verdict::Reject(ERR_LIMIT);
    }
    Verdict::Accept
}

/// Hoechstens eine laufende Anfrage je Geraet.
#[derive(Debug, Default)]
pub struct Busy {
    running: HashMap<String, String>,
}

impl Busy {
    pub fn is_busy(&self, device_id: &str) -> bool {
        self.running.contains_key(device_id)
    }

    /// `false`, wenn das Geraet schon eine Anfrage laufen hat.
    pub fn try_begin(&mut self, device_id: &str, request_id: &str) -> bool {
        if self.running.contains_key(device_id) {
            return false;
        }
        self.running
            .insert(device_id.to_string(), request_id.to_string());
        true
    }

    pub fn end(&mut self, device_id: &str, request_id: &str) {
        if self.running.get(device_id).map(String::as_str) == Some(request_id) {
            self.running.remove(device_id);
        }
    }
}

// --------------------------------------------------------- Text-Fortschritt --

/// Schneidet an einer Zeichengrenze auf hoechstens `max` Bytes.
pub fn clip_text(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Schreibt den bisher erzeugten Text mit steigender `seq` fort und signiert
/// jede Fassung (R3).
pub struct Progress {
    request_id: String,
    device_id: String,
    key: Vec<u8>,
    seq: u32,
    sent_len: usize,
    preview: bool,
}

impl Progress {
    pub fn new(request_id: &str, device_id: &str, key: &[u8]) -> Self {
        Self {
            request_id: request_id.to_string(),
            device_id: device_id.to_string(),
            key: key.to_vec(),
            seq: 0,
            sent_len: 0,
            preview: false,
        }
    }

    /// Jede weitere Antwort traegt `preview: true` (Abo-Vorschau).
    pub fn previewing(mut self) -> Self {
        self.preview = true;
        self
    }

    fn build(
        &mut self,
        text: &str,
        model_tuple: &str,
        done: bool,
        error: Option<&str>,
        usage: Option<Usage>,
    ) -> RespPayload {
        self.seq += 1;
        self.sent_len = text.len();
        let mut r = RespPayload {
            v: PAYLOAD_VERSION,
            request_id: self.request_id.clone(),
            device_id: self.device_id.clone(),
            seq: self.seq,
            model_tuple: model_tuple.to_string(),
            text: text.to_string(),
            done,
            error: error.map(str::to_string),
            usage,
            preview: self.preview,
            mac: String::new(),
        };
        r.mac = mac_hex(&self.key, resp_canonical(&r).as_bytes());
        r
    }

    /// Lebenszeichen beim Annehmen: `seq 1`, leerer Text, `model_tuple "pending"`.
    pub fn start(&mut self) -> RespPayload {
        self.build("", MODEL_PENDING, false, None, None)
    }

    /// Zwischenstand, nur wenn der Text seit dem letzten Schreiben gewachsen ist.
    pub fn update(&mut self, text: &str) -> Option<RespPayload> {
        let text = clip_text(text, MAX_RESP_BYTES);
        if text.len() <= self.sent_len {
            return None;
        }
        Some(self.build(text, MODEL_PENDING, false, None, None))
    }

    /// Letzter Stand: `done:true`, bei einem Fehler mit Code.
    pub fn finish(
        &mut self,
        text: &str,
        model_tuple: &str,
        error: Option<&str>,
        usage: Option<Usage>,
    ) -> RespPayload {
        let text = clip_text(text, MAX_RESP_BYTES);
        self.build(text, model_tuple, true, error, usage)
    }
}

/// Fehlertext des LLM-Clients auf einen Code der Spec; der Text selbst
/// (koennte Teile der Anfrage enthalten) geht nirgends hin.
pub fn map_error(err: &str) -> &'static str {
    let lower = err.to_lowercase();
    if lower.contains("cli_timeout") || lower.contains("zeitlimit") || lower.contains("timed out") {
        return ERR_TIMEOUT;
    }
    if err.starts_with(crate::managers::compliance::CODE_COMPLIANCE_BLOCKED)
        || lower.contains("monatsbudget")
    {
        return ERR_NOT_ALLOWED;
    }
    if lower.starts_with("cli_missing")
        || lower.starts_with("cli_model_not_in_plan")
        || lower.starts_with("cli_not_logged_in")
        || lower.starts_with("no_model")
        || lower.starts_with("no_provider")
    {
        return ERR_MODEL_UNAVAILABLE;
    }
    ERR_LLM_FAILED
}

// ---------------------------------------------------------------- Laufzeit --

static WAKE: Notify = Notify::const_new();

/// Weckt die Schleife (Einstellung oder Freigabe geaendert).
pub fn wake() {
    WAKE.notify_one();
}

static READY_CACHE: Mutex<Vec<(Cli, Instant, bool)>> = Mutex::new(Vec::new());

/// Bereitschaft einer CLI, fuenf Minuten gemerkt (die Pruefung startet die CLI).
async fn cli_ready(cli: Cli) -> bool {
    let cached = READY_CACHE.lock().ok().and_then(|c| {
        c.iter()
            .find(|(k, at, _)| *k == cli && at.elapsed() < READY_TTL)
            .map(|(_, _, ok)| *ok)
    });
    if let Some(ok) = cached {
        // Ein API-Schluessel in der Umgebung zaehlt sofort, nicht erst nach dem Ablauf.
        return ok && !crate::managers::llm::cli::api_key_in_env(cli);
    }
    let ok = tokio::task::spawn_blocking(move || crate::managers::llm::cli::relay_ready(cli))
        .await
        .unwrap_or(false);
    if let Ok(mut c) = READY_CACHE.lock() {
        c.retain(|(k, _, _)| *k != cli);
        c.push((cli, Instant::now(), ok));
    }
    ok
}

async fn ready_now() -> Ready {
    Ready {
        claude: cli_ready(Cli::Claude).await,
        codex: cli_ready(Cli::Codex).await,
    }
}

#[derive(Default)]
struct Shared {
    busy: Busy,
    /// Zuletzt bekannte Revision je Objekt (Basis fuer das naechste Schreiben).
    revisions: HashMap<String, i64>,
    /// Abbruchsignale laufender Anfragen, nach `request_id`.
    cancels: HashMap<String, Arc<AtomicBool>>,
}

impl Shared {
    /// Abbrechen, falls die Anfrage laeuft. `true`, wenn es eine gab.
    fn cancel(&self, request_id: &str) -> bool {
        match self.cancels.get(request_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
}

/// Der Zustand ueberlebt die Takte: jeder Takt baut einen neuen [`Ctx`], alle
/// teilen dasselbe `Arc<Mutex<Shared>>` aus dem [`LoopState`].
struct Ctx {
    cfg: SyncConfig,
    key: [u8; crypto::KEY_LEN],
    key_id: String,
    client: HubClient,
    store_path: PathBuf,
    shared: Arc<Mutex<Shared>>,
    /// App-Datenordner mit Faehigkeitstabelle und Messdatei der Abo-Vorschau.
    evidence_dir: PathBuf,
    /// Der Vorschau-Schalter live. Ohne App (Tests) „aus“: fail-closed.
    preview_switch: crate::managers::llm::preview::PreviewSwitch,
}

impl Ctx {
    /// Mit dem Schalter der App (`tick`); sonst bleibt er „aus“.
    fn with_switch(mut self, switch: crate::managers::llm::preview::PreviewSwitch) -> Self {
        self.preview_switch = switch;
        self
    }

    fn evidence(&self) -> crate::managers::llm::evidence::Evidence {
        crate::managers::llm::evidence::Evidence::load(&self.evidence_dir)
    }

    fn new(
        cfg: SyncConfig,
        key: [u8; crypto::KEY_LEN],
        store_path: PathBuf,
        shared: Arc<Mutex<Shared>>,
    ) -> Self {
        Self {
            key_id: crypto::key_id(&key),
            client: HubClient::new(&cfg.url, &cfg.token),
            key,
            cfg,
            evidence_dir: store_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default(),
            store_path,
            shared,
            preview_switch: crate::managers::llm::preview::PreviewSwitch::new(|| false),
        }
    }
}

/// Ausgang eines Schreibversuchs.
enum Written {
    Done,
    /// Das Objekt ist am Hub schon geloescht (das iPhone hat es gelesen).
    Gone,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Ctx {
    fn revision(&self, object_id: &str) -> i64 {
        self.shared
            .lock()
            .ok()
            .and_then(|s| s.revisions.get(object_id).copied())
            .unwrap_or(0)
    }

    fn remember(&self, object_id: &str, revision: i64) {
        if let Ok(mut s) = self.shared.lock() {
            s.revisions.insert(object_id.to_string(), revision);
        }
    }

    /// Ein Objekt schreiben (`Some`) oder loeschen (`None`). Bei „ueberholt"
    /// einmal mit der Revision des Hubs wiederholen. Ein schon geloeschtes
    /// Objekt wird nicht wiederbelebt (`Gone`).
    async fn put(&self, object_id: &str, plain: Option<&[u8]>) -> Result<Written, HubError> {
        let mut base = self.revision(object_id);
        for _ in 0..2 {
            let payload = match plain {
                Some(p) => crypto::seal(
                    &self.key,
                    &crypto::aad(self.cfg.user_id, COLLECTION, object_id),
                    p,
                )
                .map_err(HubError::Other)?,
                None => String::new(),
            };
            let obj = PushObject {
                collection: COLLECTION.to_string(),
                object_id: object_id.to_string(),
                base_revision: base,
                payload,
                key_id: self.key_id.clone(),
                deleted: plain.is_none(),
                blob_refs: None,
            };
            let results = self.client.push(&self.cfg.device_id, &[obj]).await?;
            let Some(r) = results.into_iter().next() else {
                return Err(HubError::Other("Hub antwortet ohne Ergebnis".into()));
            };
            match r.outcome.as_str() {
                "accepted" => {
                    self.remember(object_id, r.revision);
                    return Ok(Written::Done);
                }
                "superseded" => {
                    let Some(current) = r.current else {
                        base = 0;
                        continue;
                    };
                    self.remember(object_id, current.revision);
                    if current.deleted {
                        return Ok(if plain.is_none() {
                            Written::Done
                        } else {
                            Written::Gone
                        });
                    }
                    base = current.revision;
                }
                other => return Err(HubError::Other(format!("Hub lehnt ab: {other}"))),
            }
        }
        Err(HubError::Other(
            "Hub ueberholt das Objekt wiederholt".into(),
        ))
    }

    async fn put_json<T: Serialize>(&self, id: &str, value: &T) -> Result<Written, HubError> {
        let plain = serde_json::to_vec(value).map_err(|e| HubError::Other(e.to_string()))?;
        self.put(id, Some(&plain)).await
    }

    fn schedule_cleanup(&self, object_id: String, after_ms: i64) {
        let due = now_ms() + after_ms;
        let _ = update_store(&self.store_path, |s| {
            s.cleanup.push(PendingCleanup {
                object_id,
                due_ms: due,
            });
        });
    }

    fn decrypt<T: for<'de> Deserialize<'de>>(&self, remote: &RemoteObject) -> Option<T> {
        if remote.key_id != self.key_id {
            return None;
        }
        let plain = crypto::open(
            &self.key,
            &crypto::aad(self.cfg.user_id, &remote.collection, &remote.object_id),
            &remote.payload,
        )
        .ok()?;
        serde_json::from_slice(&plain).ok()
    }
}

/// Zustand der Schleife zwischen zwei Takten.
#[derive(Default)]
struct LoopState {
    /// Konto, fuer das der Zustand gilt (Geraet + Benutzer + Hub).
    account: Option<String>,
    /// Fingerabdruck der veroeffentlichten Liste je Geraet (`device_key`).
    published_lists: HashMap<String, String>,
    /// Alle Modelle, die mindestens ein Geraet nutzen darf.
    published: Vec<ModelEntry>,
    /// Beim Start geprueft, ob eine alte Modellliste im Hub steht (Relay aus).
    swept: bool,
    cursor: Option<i64>,
    devices: Vec<HubDevice>,
    devices_at: Option<Instant>,
    /// Laufende Anfragen, Revisionen und Abbruchsignale ueber alle Takte.
    shared: Arc<Mutex<Shared>>,
}

/// Hintergrundschleife; vom Sync-Start aus gestartet. Beobachtet die
/// Einstellung, veroeffentlicht die Modellliste und beantwortet Anfragen.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(8)).await;
        let mut st = LoopState::default();
        loop {
            let delay = tick(&app, &mut st).await;
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = WAKE.notified() => {}
            }
        }
    });
}

fn account_tag(cfg: &SyncConfig) -> String {
    format!(
        "{}|{}|{}|{}",
        cfg.url, cfg.device_id, cfg.user_id, cfg.token
    )
}

async fn tick(app: &AppHandle, st: &mut LoopState) -> Duration {
    let Some(cfg) = account::load(app) else {
        *st = LoopState::default();
        return IDLE_TICK;
    };
    let Ok(store_path) = store_path(app) else {
        return IDLE_TICK;
    };
    let tag = account_tag(&cfg);
    if st.account.as_deref() != Some(tag.as_str()) {
        *st = LoopState::default();
        st.account = Some(tag);
    }
    let Ok(key) = cfg.key() else {
        return IDLE_TICK;
    };
    // Laufende Anfragen und Revisionen leben ueber die Takte hinweg: jeder
    // Takt bekommt denselben geteilten Zustand.
    let switch_app = app.clone();
    let ctx = Arc::new(
        Ctx::new(cfg, key, store_path, st.shared.clone()).with_switch(
            crate::managers::llm::preview::PreviewSwitch::new(move || {
                crate::settings::book_talk_preview_switch_on(&switch_app)
            }),
        ),
    );
    let settings = crate::settings::get_settings(app);
    match run_tick(&ctx, st, &settings).await {
        Ok(delay) => delay,
        Err(HubError::Unauthorized) => IDLE_TICK,
        Err(e) => {
            log::warn!("Buch-Relay: Hub-Fehler ({e})");
            ERROR_TICK
        }
    }
}

async fn run_tick(
    ctx: &Arc<Ctx>,
    st: &mut LoopState,
    settings: &AppSettings,
) -> Result<Duration, HubError> {
    let now = now_ms();

    // Dauerhafter Zustand: abgelaufene Eintraege weg, faellige Loeschungen holen.
    let (store, due) = update_store(&ctx.store_path, |s| {
        s.prune(now);
        let (due, keep): (Vec<_>, Vec<_>) = s.cleanup.drain(..).partition(|c| c.due_ms <= now);
        s.cleanup = keep;
        (s.clone(), due)
    })
    .map_err(HubError::Other)?;
    let active_grants = store.grants.iter().any(|g| g.active(now));

    let wanted = settings.book_talk_relay_enabled && active_grants;
    let eligible = if wanted {
        offered_models(settings, &ctx.evidence()).await
    } else {
        Vec::new()
    };
    let models = published_models(&eligible, &store.grants, now);
    // Je freigegebenem Geraet eine eigene, mit dessen Schluessel signierte Liste.
    let lists = device_lists(&eligible, &store.grants, now);

    if !wanted || models.is_empty() {
        // Ausgeschaltet, ohne Freigabe oder ohne bereites Modell: Listen zurueckziehen (R10).
        let stale: Vec<String> = st.published_lists.drain().map(|(k, _)| k).collect();
        let had_lists = !stale.is_empty();
        for key in stale {
            ctx.put(&list_object_id(&ctx.cfg.device_id, &key), None)
                .await?;
        }
        if had_lists {
            st.published.clear();
            st.cursor = None;
        } else if !st.swept {
            sweep_lists(ctx, st, &BTreeSet::new()).await?;
        }
        st.swept = true;
        run_cleanup(ctx, due).await;
        return Ok(IDLE_TICK);
    }
    if !st.swept {
        sweep_lists(ctx, st, &lists.keys().cloned().collect()).await?;
        st.swept = true;
    }

    // Modellliste je Geraet: bei Aenderung neu veroeffentlichen, bei Wegfall loeschen.
    let gone: Vec<String> = st
        .published_lists
        .keys()
        .filter(|k| !lists.contains_key(*k))
        .cloned()
        .collect();
    for key in gone {
        ctx.put(&list_object_id(&ctx.cfg.device_id, &key), None)
            .await?;
        st.published_lists.remove(&key);
    }
    for (dkey, (grant, entries)) in &lists {
        let fingerprint = models_fingerprint(&grant.key_id, entries);
        if st.published_lists.get(dkey) == Some(&fingerprint) {
            continue;
        }
        let Some(key) = grant.key() else {
            continue;
        };
        let payload = signed_models(&ctx.cfg.device_id, grant, &key, entries.clone(), now);
        ctx.put_json(&list_object_id(&ctx.cfg.device_id, dkey), &payload)
            .await?;
        st.published_lists.insert(dkey.clone(), fingerprint);
        log::info!("Buch-Relay: {} Modell(e) veroeffentlicht", entries.len());
    }
    st.published = models;
    run_cleanup(ctx, due).await;

    // Leichte Abfrage: nur Aenderungen seit dem eigenen Cursor, ausgewertet wird nur diese Sammlung.
    let mut since = match st.cursor {
        Some(c) => c,
        None => {
            let status = ctx.client.status().await?;
            st.devices = status.devices;
            st.devices_at = Some(Instant::now());
            status.hub_seq
        }
    };
    for _ in 0..20 {
        let page = match ctx.client.pull_shared(since, 100).await {
            Ok(p) => p,
            Err(HubError::CursorExpired) => {
                st.cursor = None;
                return Ok(FAST_TICK);
            }
            Err(e) => return Err(e),
        };
        for remote in &page.objects {
            handle_remote(ctx, st, settings, remote).await;
        }
        let moved = page.cursor > since;
        since = page.cursor.max(since);
        st.cursor = Some(since);
        if !page.more || !moved {
            break;
        }
    }
    st.cursor = Some(since);
    Ok(FAST_TICK)
}

/// Faellige Loeschungen ausfuehren; was scheitert, kommt wieder auf die Liste.
async fn run_cleanup(ctx: &Arc<Ctx>, due: Vec<PendingCleanup>) {
    for item in due {
        if let Err(e) = ctx.put(&item.object_id, None).await {
            log::warn!("Buch-Relay: Aufraeumen fehlgeschlagen ({e})");
            let _ = update_store(&ctx.store_path, |s| s.cleanup.push(item));
        }
    }
}

async fn handle_remote(
    ctx: &Arc<Ctx>,
    st: &mut LoopState,
    settings: &AppSettings,
    remote: &RemoteObject,
) {
    if remote.collection != COLLECTION || remote.device_id == ctx.cfg.device_id {
        return;
    }
    let Some((kind, id)) = parse_object_id(&remote.object_id) else {
        return;
    };
    match kind {
        Kind::Resp => {
            // Das iPhone hat die Antwort gelesen und geloescht -- oder, solange die
            // Anfrage noch laeuft, aufgegeben: dann den CLI-Prozess beenden.
            if remote.deleted {
                cancel_request(ctx, id);
                ctx.remember(&remote.object_id, remote.revision);
                let oid = remote.object_id.clone();
                let _ = update_store(&ctx.store_path, |s| {
                    s.cleanup.retain(|c| c.object_id != oid)
                });
            }
        }
        Kind::Req if !remote.deleted => {
            handle_request(ctx, st, settings, remote, id).await;
        }
        // Das iPhone loescht `req` (Abbruch, R10): laufenden Aufruf beenden.
        Kind::Req => cancel_request(ctx, id),
        _ => {}
    }
}

/// Eine laufende Anfrage abbrechen (iPhone hat `req`/`resp` geloescht).
fn cancel_request(ctx: &Ctx, request_id: &str) {
    let cancelled = ctx
        .shared
        .lock()
        .map(|s| s.cancel(request_id))
        .unwrap_or(false);
    if cancelled {
        log::info!("Buch-Relay: Anfrage vom iPhone abgebrochen");
    }
}

async fn handle_request(
    ctx: &Arc<Ctx>,
    st: &mut LoopState,
    settings: &AppSettings,
    remote: &RemoteObject,
    request_id: &str,
) {
    let Some(req) = ctx.decrypt::<ReqPayload>(remote) else {
        log::warn!("Buch-Relay: Anfrage nicht lesbar");
        return;
    };
    // Das Geraet muss auch in der Geraeteliste des Hubs stehen (Konto desselben Benutzers).
    if !device_listed(ctx, st, &remote.device_id).await {
        log::info!("Buch-Relay: Anfrage uebergangen (Geraet unbekannt)");
        return;
    }
    let now = now_ms();
    let outcome = update_store(&ctx.store_path, |store| {
        let busy = ctx
            .shared
            .lock()
            .map(|s| s.busy.is_busy(&remote.device_id))
            .unwrap_or(true);
        let gate = Gate {
            now_ms: now,
            hub_device_id: &remote.device_id,
            published: &st.published,
            busy,
            store,
        };
        let mut verdict = check_request(request_id, &req, &gate);
        let dkey = device_key(&req.device_id);
        // Erste authentische Anfrage: Freigabe aktivieren, der Code ist verbraucht.
        if matches!(verdict, Verdict::Accept | Verdict::Reject(_))
            && store.activate(&req.device_id, &req.key_id)
        {
            log::info!("Buch-Relay: Freigabe aktiviert (erste gueltige Anfrage)");
        }
        let grant = store.grant_for(&req.device_id, &req.key_id, now).cloned();
        match verdict {
            Verdict::Accept => {
                let began = ctx
                    .shared
                    .lock()
                    .map(|mut s| {
                        let ok = s.busy.try_begin(&remote.device_id, request_id);
                        if ok {
                            s.cancels
                                .insert(request_id.to_string(), Arc::new(AtomicBool::new(false)));
                        }
                        ok
                    })
                    .unwrap_or(false);
                if began {
                    // Dauerhaft vermerken, bevor irgendetwas laeuft (R2).
                    store.record(request_id, &dkey, true, req.expires_at * 1000, now);
                } else {
                    verdict = Verdict::Reject(ERR_LIMIT);
                    store.record(request_id, &dkey, false, req.expires_at * 1000, now);
                }
            }
            Verdict::Reject(_) => {
                store.record(request_id, &dkey, false, req.expires_at * 1000, now)
            }
            Verdict::Ignore(_) => {}
        }
        (verdict, grant)
    });
    let Ok((verdict, grant)) = outcome else {
        log::warn!("Buch-Relay: Zustand nicht gespeichert, Anfrage verworfen");
        if let Ok(mut s) = ctx.shared.lock() {
            s.busy.end(&remote.device_id, request_id);
        }
        return;
    };
    let ctx = ctx.clone();
    let req_object = remote.object_id.clone();
    let config_id = st
        .published
        .iter()
        .find(|m| m.id == req.model_id)
        .map(|m| m.config_id.clone());
    match (verdict, grant.and_then(|g| g.key())) {
        (Verdict::Accept, Some(key)) => {
            let settings = settings.clone();
            let sender = remote.device_id.clone();
            let config_id = config_id.unwrap_or_default();
            tauri::async_runtime::spawn(async move {
                // R5: die Anfrage verschwindet beim Annehmen aus dem Hub.
                let _ = ctx.put(&req_object, None).await;
                process(&ctx, &settings, &config_id, req, &sender, &key).await;
            });
        }
        (Verdict::Reject(code), Some(key)) => {
            log::info!("Buch-Relay: Anfrage abgewiesen ({code})");
            tauri::async_runtime::spawn(async move {
                let _ = ctx.put(&req_object, None).await;
                let mut progress = Progress::new(&req.request_id, &req.device_id, &key);
                finish(&ctx, &mut progress, "", MODEL_UNKNOWN, Some(code), None).await;
            });
        }
        (Verdict::Ignore(why), _) => {
            log::info!("Buch-Relay: Anfrage uebergangen ({why})");
            if matches!(why, "frist") {
                tauri::async_runtime::spawn(async move {
                    let _ = ctx.put(&req_object, None).await;
                });
            }
        }
        _ => {}
    }
}

/// Steht das Geraet in der Geraeteliste des Hubs? Ein Unbekannter loest
/// hoechstens alle drei Sekunden eine Auffrischung aus.
async fn device_listed(ctx: &Ctx, st: &mut LoopState, device_id: &str) -> bool {
    let listed = |devices: &[HubDevice]| {
        let key = device_key(device_id);
        !device_id.is_empty() && devices.iter().any(|d| d.device == key)
    };
    if listed(&st.devices) {
        return true;
    }
    if st.devices_at.is_some_and(|t| t.elapsed() < DEVICES_REFRESH) {
        return false;
    }
    match ctx.client.status().await {
        Ok(status) => {
            st.devices = status.devices;
            st.devices_at = Some(Instant::now());
            listed(&st.devices)
        }
        Err(_) => false,
    }
}

/// Ist die Freigabe noch gueltig (nicht widerrufen, nicht abgelaufen)?
fn still_authorized(path: &Path, device_id: &str, key_id: &str) -> bool {
    load_store(path)
        .grant_for(device_id, key_id, now_ms())
        .is_some()
}

/// Eine Anfrage beantworten: Modell isoliert aufrufen, den Text jede Sekunde
/// zurueckschreiben, am Ende `done`, `resp` zum Loeschen vormerken.
#[allow(deprecated)]
async fn process(
    ctx: &Arc<Ctx>,
    settings: &AppSettings,
    config_id: &str,
    req: ReqPayload,
    sender: &str,
    key: &[u8],
) {
    use crate::llm_client::{
        CliMeta, StreamMessage, CLI_CANCEL, CLI_MAX_TOKENS, CLI_META, CLI_PREVIEW,
        CLI_PREVIEW_SWITCH, CLI_REQUIRE_USAGE, CLI_TIMEOUT,
    };
    let mut progress = Progress::new(&req.request_id, &req.device_id, key);
    // Das Abbruchsignal gehoert dieser Anfrage; verlassen wir `process` auf
    // irgendeinem Weg, wird es gesetzt und ein noch laufender CLI-Prozess beendet.
    let cancel = ctx
        .shared
        .lock()
        .ok()
        .and_then(|s| s.cancels.get(&req.request_id).cloned())
        .unwrap_or_default();
    let _cancel_on_exit = CancelOnDrop(cancel.clone());
    // Lebenszeichen vor der 15-s-Frist des iPhones.
    let started = progress.start();
    if let Ok(Written::Gone) = ctx
        .put_json(&object_id(Kind::Resp, &req.request_id), &started)
        .await
    {
        end_busy(ctx, sender, &req.request_id);
        return;
    }
    let call = settings_for_model(settings, config_id);
    let resolved = crate::managers::meetings::llm_call::resolve_provider_coded(&call);
    let (provider, model, api_key) = match resolved {
        // Nur Abo-Pfade (R6): kein API-Schluessel in der Umgebung der CLI.
        Ok(r) => match Cli::from_base_url(&r.0.base_url) {
            Some(cli) if !crate::managers::llm::cli::api_key_in_env(cli) => r,
            _ => {
                finish(
                    ctx,
                    &mut progress,
                    "",
                    MODEL_UNKNOWN,
                    Some(ERR_MODEL_UNAVAILABLE),
                    None,
                )
                .await;
                end_busy(ctx, sender, &req.request_id);
                return;
            }
        },
        Err(_) => {
            finish(
                ctx,
                &mut progress,
                "",
                MODEL_UNKNOWN,
                Some(ERR_MODEL_UNAVAILABLE),
                None,
            )
            .await;
            end_busy(ctx, sender, &req.request_id);
            return;
        }
    };
    // Abo-CLI nur als Vorschau (lva#334): Schalter, gueltiges Tupel, belegter
    // Kontext; sonst wird nichts aufgerufen.
    let preview_check = match Cli::from_base_url(&provider.base_url) {
        Some(cli) => {
            let id = model.clone();
            let current = tokio::task::spawn_blocking(move || {
                crate::managers::llm::preview::current_binding(cli, &id)
            })
            .await
            .ok()
            .and_then(Result::ok);
            crate::book_ai::talk::preview_check(
                &call,
                cli,
                current,
                req.max_tokens,
                &ctx.evidence(),
            )
        }
        None => None,
    };
    let Some(preview_check) = preview_check else {
        finish(
            ctx,
            &mut progress,
            "",
            MODEL_UNKNOWN,
            Some(ERR_MODEL_UNAVAILABLE),
            None,
        )
        .await;
        end_busy(ctx, sender, &req.request_id);
        return;
    };
    progress = progress.previewing();
    let mut messages = Vec::new();
    if !req.system.trim().is_empty() {
        messages.push(StreamMessage::new("system", req.system.clone()));
    }
    messages.push(StreamMessage::new("user", req.prompt.clone()));

    let buffer = Arc::new(Mutex::new(String::new()));
    let sink = buffer.clone();
    let on_delta = move |delta: &str| {
        if let Ok(mut b) = sink.lock() {
            b.push_str(delta);
        }
    };
    let meta = Arc::new(Mutex::new(CliMeta::default()));
    // Dasselbe Gesamtbudget und dieselbe Pruefung wie am Desktop: die Vorab-
    // Pruefung und die Nutzungsangabe erzwingt `llm_versand::transport::cli` (`call_with_options`), der
    // Text kommt erst nach der Pruefung in den Puffer.
    let llm = CLI_PREVIEW_SWITCH.scope(
        ctx.preview_switch.clone(),
        CLI_PREVIEW.scope(
            preview_check,
            CLI_REQUIRE_USAGE.scope(
                true,
                CLI_META.scope(
                    meta.clone(),
                    CLI_TIMEOUT.scope(
                        CALL_TIMEOUT,
                        CLI_CANCEL.scope(
                            cancel.clone(),
                            CLI_MAX_TOKENS.scope(
                                req.max_tokens,
                                crate::llm_versand::uebergang::send_chat_completion_stream(
                                    crate::managers::usage::Purpose::Chat,
                                    &provider,
                                    api_key,
                                    &model,
                                    messages,
                                    &on_delta,
                                ),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );
    let guarded = tokio::time::timeout(CALL_TIMEOUT + Duration::from_secs(5), llm);
    tokio::pin!(guarded);
    let mut ticker = tokio::time::interval(RESP_INTERVAL);
    let outcome = loop {
        tokio::select! {
            r = &mut guarded => break r,
            _ = ticker.tick() => {
                // Vorschau waehrend der Anfrage ausgeschaltet: Prozessbaum beenden
                // (Signal, `llm_versand::transport::cli` beobachtet den Schalter auch
                // selbst), Ergebnis verwerfen, dem iPhone nur den Fehler melden.
                if !ctx.preview_switch.is_on() {
                    cancel.store(true, Ordering::SeqCst);
                    log::info!("Buch-Relay: Vorschau ausgeschaltet, laufende Anfrage verworfen");
                    finish(ctx, &mut progress, "", MODEL_UNKNOWN, Some(ERR_MODEL_UNAVAILABLE), None).await;
                    end_busy(ctx, sender, &req.request_id);
                    return;
                }
                // Widerruf waehrend der Anfrage (R10): nichts mehr schreiben, Ergebnis verwerfen.
                if !still_authorized(&ctx.store_path, &req.device_id, &req.key_id) {
                    log::info!("Buch-Relay: Freigabe widerrufen, laufende Anfrage verworfen");
                    end_busy(ctx, sender, &req.request_id);
                    return;
                }
                let text = buffer.lock().map(|b| b.clone()).unwrap_or_default();
                if let Some(update) = progress.update(&text) {
                    if let Ok(Written::Gone) = ctx.put_json(&object_id(Kind::Resp, &req.request_id), &update).await {
                        end_busy(ctx, sender, &req.request_id);
                        return;
                    }
                }
            }
        }
    };
    // Abbruch durch das iPhone (req/resp geloescht): nichts mehr schreiben.
    if cancel.load(Ordering::SeqCst)
        || !still_authorized(&ctx.store_path, &req.device_id, &req.key_id)
    {
        end_busy(ctx, sender, &req.request_id);
        return;
    }
    // Vor der Ausgabe (Relay-Antwort): Schalter aus heisst verwerfen, kein Text.
    if !ctx.preview_switch.is_on() {
        log::info!("Buch-Relay: Vorschau ausgeschaltet, Ergebnis verworfen");
        finish(
            ctx,
            &mut progress,
            "",
            MODEL_UNKNOWN,
            Some(ERR_MODEL_UNAVAILABLE),
            None,
        )
        .await;
        end_busy(ctx, sender, &req.request_id);
        return;
    }
    let info = meta.lock().map(|m| m.clone()).unwrap_or_default();
    // R7: das Modell laut CLI-Ausgabe; nennt sie keines, melden wir `unknown`
    // (nie die angefragte Kennung: das iPhone behandelt `unknown` als ungeprueft).
    let model_tuple = info
        .model
        .clone()
        .unwrap_or_else(|| MODEL_UNKNOWN.to_string());
    let usage = (info.prompt_tokens + info.completion_tokens > 0).then(|| Usage {
        input_tokens: info.prompt_tokens,
        output_tokens: info.completion_tokens,
    });
    let partial = buffer.lock().map(|b| b.clone()).unwrap_or_default();
    match outcome {
        Ok(Ok(text)) => finish(ctx, &mut progress, &text, &model_tuple, None, usage).await,
        Ok(Err(e)) => {
            let code = map_error(&e);
            log::warn!("Buch-Relay: Modellaufruf fehlgeschlagen ({code})");
            finish(
                ctx,
                &mut progress,
                &partial,
                &model_tuple,
                Some(code),
                usage,
            )
            .await;
        }
        Err(_) => {
            log::warn!("Buch-Relay: Modellaufruf ueberschreitet den Zeitrahmen");
            finish(
                ctx,
                &mut progress,
                &partial,
                &model_tuple,
                Some(ERR_TIMEOUT),
                usage,
            )
            .await;
        }
    }
    end_busy(ctx, sender, &req.request_id);
}

/// Letzten Stand schreiben und `resp` zum Loeschen vormerken.
async fn finish(
    ctx: &Ctx,
    progress: &mut Progress,
    text: &str,
    model_tuple: &str,
    error: Option<&str>,
    usage: Option<Usage>,
) {
    let resp_id = object_id(Kind::Resp, &progress.request_id);
    let last = progress.finish(text, model_tuple, error, usage);
    if let Err(e) = ctx.put_json(&resp_id, &last).await {
        log::warn!("Buch-Relay: Antwort nicht geschrieben ({e})");
    }
    ctx.schedule_cleanup(resp_id, RESP_KEEP_MS);
}

fn end_busy(ctx: &Ctx, sender: &str, request_id: &str) {
    if let Ok(mut s) = ctx.shared.lock() {
        s.busy.end(sender, request_id);
        s.cancels.remove(request_id);
    }
}

/// Setzt das Abbruchsignal beim Verlassen von `process`.
struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------- Befehle --

/// Schalter „iPhone darf Abo-Modelle nutzen" (Einstellungen -> Modelle je
/// Funktion). Veroeffentlicht oder loescht die Modellliste sofort. Ohne
/// Freigabe je Geraet ([`booktalk_relay_authorize`]) bleibt sie leer.
#[tauri::command]
#[specta::specta]
pub fn change_book_talk_relay_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.book_talk_relay_enabled = enabled;
    crate::settings::write_settings(&app, settings);
    wake();
    Ok(())
}

/// Eine Freigabe, wie die Oberflaeche sie zeigt (ohne Schluessel).
#[derive(Serialize, Debug, Clone, specta::Type)]
pub struct RelayGrantInfo {
    pub device_key: String,
    pub name: String,
    pub key_id: String,
    pub model_ids: Vec<String>,
    pub created_at_ms: f64,
    /// Ablauf der Freigabe (90 Tage).
    pub expires_at_ms: f64,
    /// `true`, solange das iPhone den Einrichtungscode noch nicht benutzt hat.
    pub pending: bool,
    /// Ablauf des Einrichtungscodes (10 Minuten ab Erzeugung).
    pub code_expires_at_ms: f64,
}

/// Einrichtungscode zum Anzeigen: Text (Rueckfall), QR-Bild (SVG) und Fristen.
#[derive(Serialize, Debug, Clone, specta::Type)]
pub struct RelayPairing {
    pub code: String,
    pub qr_svg: String,
    pub code_expires_at_ms: f64,
    pub grant_expires_at_ms: f64,
}

/// Ein Modell, das am PC fuer das Relay in Frage kommt.
#[derive(Serialize, Debug, Clone, specta::Type)]
pub struct RelayModelInfo {
    pub id: String,
    pub label: String,
    pub provider: String,
}

#[tauri::command]
#[specta::specta]
pub fn booktalk_relay_grants(app: AppHandle) -> Result<Vec<RelayGrantInfo>, String> {
    let store = load_store(&store_path(&app)?);
    let now = now_ms();
    Ok(store
        .grants
        .iter()
        .filter(|g| g.active(now))
        .map(|g| RelayGrantInfo {
            device_key: g.device_key.clone(),
            name: g.name.clone(),
            key_id: g.key_id.clone(),
            model_ids: g.model_ids.clone(),
            created_at_ms: g.created_at_ms as f64,
            expires_at_ms: g.expires_at_ms as f64,
            pending: g.pending(now),
            code_expires_at_ms: g.code_expires_at_ms as f64,
        })
        .collect())
}

/// Die Abo-Modelle dieses PCs, die ein iPhone nutzen koennte (bereite CLIs).
#[tauri::command]
#[specta::specta]
pub async fn booktalk_relay_models(app: AppHandle) -> Result<Vec<RelayModelInfo>, String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.book_talk_relay_enabled = true;
    Ok(eligible_models(&settings, ready_now().await)
        .into_iter()
        .map(|m| RelayModelInfo {
            id: m.id,
            label: m.label,
            provider: m.provider,
        })
        .collect())
}

/// Die volle Geraete-Kennung zu einer Kurzkennung aus der Geraeteliste des Hubs:
/// steht in den Objekten, die das Geraet gepusht hat. Einmalig bei der Freigabe.
async fn resolve_device_id(client: &HubClient, key: &str) -> Result<Option<String>, HubError> {
    let mut since = 0;
    for _ in 0..200 {
        let page = client.pull_shared(since, 100).await?;
        if let Some(o) = page
            .objects
            .iter()
            .find(|o| !o.device_id.is_empty() && device_key(&o.device_id) == key)
        {
            return Ok(Some(o.device_id.clone()));
        }
        if !page.more || page.cursor <= since {
            break;
        }
        since = page.cursor;
    }
    Ok(None)
}

fn pairing_of(cfg: &SyncConfig, grant: &DeviceGrant) -> Result<RelayPairing, String> {
    let code = pairing_code(&cfg.device_id, &cfg.device_name, grant);
    Ok(RelayPairing {
        qr_svg: qr_svg(&code)?,
        code,
        code_expires_at_ms: grant.code_expires_at_ms as f64,
        grant_expires_at_ms: grant.expires_at_ms as f64,
    })
}

/// Ein iPhone freigeben (R1): neuer Relay-Schluessel, die gewaehlten Modelle,
/// 90 Tage. Gibt den Einrichtungscode (Text und QR) zurueck, den das iPhone
/// einliest. Die Freigabe ist erst nach der ersten gueltigen Anfrage des iPhones
/// aktiv; ohne sie verfaellt sie samt Schluessel nach 10 Minuten.
#[tauri::command]
#[specta::specta]
pub async fn booktalk_relay_authorize(
    app: AppHandle,
    device_key: String,
    name: String,
    model_ids: Vec<String>,
) -> Result<RelayPairing, String> {
    if !valid_device_key(&device_key) {
        return Err("Ungueltige Geraetekennung.".into());
    }
    if name.trim().is_empty() {
        return Err("Bitte dem Geraet einen Namen geben.".into());
    }
    let cfg = account::load(&app).ok_or("Nicht angemeldet")?;
    let mut settings = crate::settings::get_settings(&app);
    settings.book_talk_relay_enabled = true;
    let offered: Vec<String> = eligible_models(&settings, ready_now().await)
        .into_iter()
        .map(|m| m.id)
        .collect();
    let chosen: Vec<String> = model_ids
        .into_iter()
        .filter(|id| offered.contains(id))
        .collect();
    if chosen.is_empty() {
        return Err("Mindestens ein bereites Abo-Modell auswaehlen.".into());
    }
    let client = HubClient::new(&cfg.url, &cfg.token);
    let device_id = resolve_device_id(&client, &device_key)
        .await
        .map_err(|e| e.to_string())?
        .ok_or(
            "Dieses Geraet hat noch nichts abgeglichen; bitte zuerst am iPhone synchronisieren.",
        )?;
    let grant = new_grant(&device_id, name.trim(), chosen, now_ms());
    let pairing = pairing_of(&cfg, &grant)?;
    update_store(&store_path(&app)?, |s| s.add_grant(grant))?;
    wake();
    Ok(pairing)
}

/// Einrichtungscode noch einmal zeigen: erzeugt einen **neuen** Schluessel
/// (der alte ist sofort ungueltig), die Freigabe ist wieder `pending`, der
/// Code gilt wieder 10 Minuten. Name, Modelle und Geraet bleiben.
#[tauri::command]
#[specta::specta]
pub fn booktalk_relay_renew_code(
    app: AppHandle,
    device_key: String,
) -> Result<RelayPairing, String> {
    let cfg = account::load(&app).ok_or("Nicht angemeldet")?;
    let now = now_ms();
    let grant = update_store(&store_path(&app)?, |s| {
        let old = s
            .grants
            .iter()
            .find(|g| g.device_key == device_key && g.expires_at_ms > now)
            .cloned()?;
        let fresh = new_grant(&old.device_id, &old.name, old.model_ids.clone(), now);
        s.add_grant(fresh.clone());
        Some(fresh)
    })?
    .ok_or("Keine Freigabe fuer dieses Geraet.")?;
    let pairing = pairing_of(&cfg, &grant)?;
    wake();
    Ok(pairing)
}

/// Freigabe entziehen (R10): Schluessel loeschen, Modellliste neu bewerten
/// (leer = zurueckziehen); laufende Anfragen werden beim naechsten Takt verworfen.
#[tauri::command]
#[specta::specta]
pub fn booktalk_relay_revoke(app: AppHandle, device_key: String) -> Result<(), String> {
    update_store(&store_path(&app)?, |s| s.revoke(&device_key))?;
    wake();
    Ok(())
}

#[cfg(test)]
mod tests;
