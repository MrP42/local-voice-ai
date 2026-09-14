//! Verbrauchs-Ledger: jeder Sprachmodell-Aufruf wird gebucht -- Zeitpunkt,
//! Zweck, Anbieter, Modell, Token, Kosten.
//!
//! Die Preise werden je Ereignis eingefroren: aendert der Nutzer spaeter den
//! Preis eines Modells, bleibt die Historie, was sie damals gekostet hat.
//! Kosten liegen als ganze Mikro-Dollar in der Datenbank, nie als Float --
//! Summen ueber tausend Ereignisse sollen auf den Cent stimmen.
//!
//! Das Buchen darf eine Antwort nie verzoegern oder scheitern lassen: Fehler
//! werden protokolliert, nicht weitergereicht.

use anyhow::Result;
use chrono::{DateTime, Datelike, Local, TimeZone, Utc};
use log::{info, warn};
use rusqlite::{params, Connection};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::AppHandle;

use crate::settings::{AppSettings, LlmConnection, LlmModelConfig, PostProcessProvider};

static MIGRATIONS: &[M] = &[M::up(
    "CREATE TABLE IF NOT EXISTS usage_event (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        ts INTEGER NOT NULL,
        purpose TEXT NOT NULL,
        connection_id TEXT NOT NULL,
        connection_kind TEXT NOT NULL,
        connection_label TEXT NOT NULL,
        model_id TEXT NOT NULL,
        model_label TEXT NOT NULL,
        prompt_tokens INTEGER NOT NULL,
        completion_tokens INTEGER NOT NULL,
        price_input_per_mtok REAL,
        price_output_per_mtok REAL,
        cost_micro INTEGER NOT NULL,
        duration_ms INTEGER NOT NULL,
        ok INTEGER NOT NULL,
        error TEXT
    );
    CREATE INDEX IF NOT EXISTS usage_event_ts ON usage_event(ts);
    CREATE INDEX IF NOT EXISTS usage_event_conn_ts ON usage_event(connection_id, ts);",
)];

/// Wofuer ein Aufruf war. Bewusst eine feste Liste: die Auswertung gruppiert
/// danach, und ein freier Text wuerde in zehn Schreibweisen zerfallen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    PostProcess,
    Minutes,
    Summary,
    Tagging,
    Translation,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::PostProcess => "post_process",
            Purpose::Minutes => "minutes",
            Purpose::Summary => "summary",
            Purpose::Tagging => "tagging",
            Purpose::Translation => "translation",
        }
    }
}

/// Token-Zaehler einer Antwort, wie der Anbieter sie meldet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Ein gebuchter Aufruf.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UsageEvent {
    pub id: i64,
    /// Unix-Sekunden.
    pub ts: i64,
    pub purpose: String,
    pub connection_id: String,
    pub connection_kind: String,
    pub connection_label: String,
    pub model_id: String,
    pub model_label: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub price_input_per_mtok: Option<f64>,
    pub price_output_per_mtok: Option<f64>,
    /// Kosten in Mikro-Dollar (1 USD = 1 000 000).
    pub cost_micro: i64,
    pub duration_ms: u32,
    pub ok: bool,
    pub error: Option<String>,
}

/// Eine Zeile der Auswertung: Summen je Modell, je Zweck oder je Tag.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UsageBucket {
    pub key: String,
    pub label: String,
    pub calls: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_micro: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct UsageSummary {
    pub range: UsageRange,
    pub calls: u32,
    pub failed: u32,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub cost_micro: i64,
    pub by_model: Vec<UsageBucket>,
    pub by_purpose: Vec<UsageBucket>,
    /// Je Kalendertag (lokale Zeit), Schluessel `JJJJ-MM-TT`, aufsteigend.
    pub by_day: Vec<UsageBucket>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum UsageRange {
    Today,
    Week,
    Month,
    All,
}

impl UsageRange {
    /// Unix-Sekunde, ab der gezaehlt wird (lokale Zeit: "heute" ist der
    /// Kalendertag des Nutzers, nicht UTC).
    fn since(self, now: DateTime<Local>) -> i64 {
        let midnight = now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|t| Local.from_local_datetime(&t).single())
            .unwrap_or(now);
        match self {
            UsageRange::Today => midnight.timestamp(),
            UsageRange::Week => midnight.timestamp() - 6 * 86_400,
            UsageRange::Month => now
                .date_naive()
                .with_day(1)
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .and_then(|t| Local.from_local_datetime(&t).single())
                .map(|t| t.timestamp())
                .unwrap_or(midnight.timestamp()),
            UsageRange::All => 0,
        }
    }
}

/// Budgetstand einer Verbindung im laufenden Kalendermonat.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BudgetState {
    pub connection_id: String,
    pub spent_micro: i64,
    /// Monatslimit in Mikro-Dollar; `None`, wenn keins gesetzt ist.
    pub limit_micro: Option<i64>,
    /// Anteil 0..∞ des Limits; `None` ohne Limit.
    pub ratio: Option<f64>,
    pub enforced: bool,
}

impl BudgetState {
    /// Ab hier warnt die Fussleiste.
    pub const WARN_RATIO: f64 = 0.8;

    pub fn warning(&self) -> bool {
        self.ratio.is_some_and(|r| r >= Self::WARN_RATIO)
    }

    pub fn exceeded(&self) -> bool {
        self.ratio.is_some_and(|r| r >= 1.0)
    }
}

/// Kosten eines Aufrufs in Mikro-Dollar: Token × Preis je Million, je
/// Richtung, kaufmaennisch gerundet. Ohne Preis kostet er null -- und das
/// ist eine Aussage ("unbekannt"), keine Luecke: die Preise stehen daneben.
pub fn cost_micro(usage: TokenUsage, price_in: Option<f64>, price_out: Option<f64>) -> i64 {
    let per = |tokens: u64, price: Option<f64>| -> f64 {
        price.map(|p| tokens as f64 * p).unwrap_or(0.0)
    };
    // Token × USD/MTok = Mikro-USD, da 1 MTok = 1e6 Token und 1 USD = 1e6 µUSD.
    (per(usage.prompt_tokens, price_in) + per(usage.completion_tokens, price_out)).round() as i64
}

pub fn micro_to_usd(micro: i64) -> f64 {
    micro as f64 / 1_000_000.0
}

fn usd_to_micro(usd: f64) -> i64 {
    (usd * 1_000_000.0).round() as i64
}

/// Was ein Aufruf zum Buchen mitbringt -- ohne Datenbank-Kennung und ohne
/// Zeit, die setzt das Ledger.
#[derive(Debug, Clone)]
pub struct NewUsageEvent {
    pub purpose: Purpose,
    pub connection_id: String,
    pub connection_kind: String,
    pub connection_label: String,
    pub model_id: String,
    pub model_label: String,
    pub usage: TokenUsage,
    pub price_input_per_mtok: Option<f64>,
    pub price_output_per_mtok: Option<f64>,
    pub duration_ms: u32,
    pub ok: bool,
    pub error: Option<String>,
}

impl NewUsageEvent {
    /// Baut das Ereignis aus dem, was der Aufrufer weiss (Anbieter-Vorlage,
    /// Modellname) und dem, was die Einstellungen dazu sagen (Verbindung,
    /// Preise). Passt das aktive Modell zum Aufruf, werden dessen Preise
    /// eingefroren; sonst wird ohne Preis gebucht -- ehrlich, nicht geraten.
    pub fn from_call(
        purpose: Purpose,
        provider: &PostProcessProvider,
        remote_model: &str,
        active: Option<(&LlmConnection, &LlmModelConfig)>,
        usage: TokenUsage,
        duration_ms: u32,
        result: Result<(), String>,
    ) -> Self {
        let matching = active.filter(|(c, m)| c.kind == provider.id && m.remote_id == remote_model);
        let (connection_id, connection_kind, connection_label, model_id, model_label, pin, pout) =
            match matching {
                Some((c, m)) => (
                    c.id.clone(),
                    c.kind.clone(),
                    c.label.clone(),
                    m.id.clone(),
                    m.label.clone(),
                    m.price_input_per_mtok,
                    m.price_output_per_mtok,
                ),
                None => (
                    provider.id.clone(),
                    provider.id.clone(),
                    provider.label.clone(),
                    LlmModelConfig::make_id(&provider.id, remote_model),
                    remote_model.to_string(),
                    None,
                    None,
                ),
            };
        let (ok, error) = match result {
            Ok(()) => (true, None),
            Err(e) => (false, Some(e)),
        };
        Self {
            purpose,
            connection_id,
            connection_kind,
            connection_label,
            model_id,
            model_label,
            usage,
            price_input_per_mtok: pin,
            price_output_per_mtok: pout,
            duration_ms,
            ok,
            error,
        }
    }
}

pub struct UsageLedger {
    conn: Mutex<Connection>,
    #[allow(dead_code)]
    path: PathBuf,
}

impl UsageLedger {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let dir = crate::portable::app_data_dir(app_handle)?;
        std::fs::create_dir_all(&dir)?;
        Self::open(&dir.join("usage.db"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        let migrations = Migrations::new(MIGRATIONS.to_vec());
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid usage migrations");
        migrations.to_latest(&mut conn)?;
        info!("Verbrauchs-Ledger geoeffnet: {}", path.display());
        Ok(Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        })
    }

    pub fn record(&self, event: NewUsageEvent) -> Result<i64> {
        self.record_at(event, Utc::now().timestamp())
    }

    pub fn record_at(&self, event: NewUsageEvent, ts: i64) -> Result<i64> {
        let cost = cost_micro(
            event.usage,
            event.price_input_per_mtok,
            event.price_output_per_mtok,
        );
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("ledger lock"))?;
        conn.execute(
            "INSERT INTO usage_event (ts, purpose, connection_id, connection_kind, connection_label,
                model_id, model_label, prompt_tokens, completion_tokens, price_input_per_mtok,
                price_output_per_mtok, cost_micro, duration_ms, ok, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                ts,
                event.purpose.as_str(),
                event.connection_id,
                event.connection_kind,
                event.connection_label,
                event.model_id,
                event.model_label,
                event.usage.prompt_tokens as i64,
                event.usage.completion_tokens as i64,
                event.price_input_per_mtok,
                event.price_output_per_mtok,
                cost,
                event.duration_ms as i64,
                event.ok as i64,
                event.error,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn events(&self, limit: u32, offset: u32) -> Result<Vec<UsageEvent>> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("ledger lock"))?;
        let mut stmt = conn.prepare(
            "SELECT id, ts, purpose, connection_id, connection_kind, connection_label, model_id,
                    model_label, prompt_tokens, completion_tokens, price_input_per_mtok,
                    price_output_per_mtok, cost_micro, duration_ms, ok, error
             FROM usage_event ORDER BY ts DESC, id DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt.query_map(params![limit as i64, offset as i64], |r| {
            Ok(UsageEvent {
                id: r.get(0)?,
                ts: r.get(1)?,
                purpose: r.get(2)?,
                connection_id: r.get(3)?,
                connection_kind: r.get(4)?,
                connection_label: r.get(5)?,
                model_id: r.get(6)?,
                model_label: r.get(7)?,
                prompt_tokens: r.get::<_, i64>(8)? as u32,
                completion_tokens: r.get::<_, i64>(9)? as u32,
                price_input_per_mtok: r.get(10)?,
                price_output_per_mtok: r.get(11)?,
                cost_micro: r.get(12)?,
                duration_ms: r.get::<_, i64>(13)? as u32,
                ok: r.get::<_, i64>(14)? != 0,
                error: r.get(15)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn summary(&self, range: UsageRange) -> Result<UsageSummary> {
        self.summary_since(range, range.since(Local::now()))
    }

    fn summary_since(&self, range: UsageRange, since: i64) -> Result<UsageSummary> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("ledger lock"))?;
        let (calls, failed, prompt, completion, cost): (i64, i64, i64, i64, i64) = conn.query_row(
            "SELECT COUNT(*), SUM(CASE WHEN ok = 0 THEN 1 ELSE 0 END),
                    COALESCE(SUM(prompt_tokens), 0), COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cost_micro), 0)
             FROM usage_event WHERE ts >= ?1",
            params![since],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ))
            },
        )?;

        let buckets = |sql: &str| -> Result<Vec<UsageBucket>> {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(params![since], |r| {
                Ok(UsageBucket {
                    key: r.get(0)?,
                    label: r.get(1)?,
                    calls: r.get::<_, i64>(2)? as u32,
                    prompt_tokens: r.get::<_, i64>(3)? as u32,
                    completion_tokens: r.get::<_, i64>(4)? as u32,
                    cost_micro: r.get(5)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        };

        let by_model = buckets(
            "SELECT model_id, MAX(model_label || ' · ' || connection_label), COUNT(*),
                    SUM(prompt_tokens), SUM(completion_tokens), SUM(cost_micro)
             FROM usage_event WHERE ts >= ?1 GROUP BY model_id ORDER BY SUM(cost_micro) DESC, COUNT(*) DESC",
        )?;
        let by_purpose = buckets(
            "SELECT purpose, purpose, COUNT(*), SUM(prompt_tokens), SUM(completion_tokens), SUM(cost_micro)
             FROM usage_event WHERE ts >= ?1 GROUP BY purpose ORDER BY COUNT(*) DESC",
        )?;
        // Tagesgrenzen in lokaler Zeit: SQLite rechnet in UTC, deshalb den
        // Versatz des Nutzers als Sekunden hineinschieben.
        let offset = Local::now().offset().local_minus_utc() as i64;
        let by_day = buckets(&format!(
            "SELECT date(ts + {offset}, 'unixepoch'), date(ts + {offset}, 'unixepoch'), COUNT(*),
                    SUM(prompt_tokens), SUM(completion_tokens), SUM(cost_micro)
             FROM usage_event WHERE ts >= ?1 GROUP BY 1 ORDER BY 1 ASC"
        ))?;

        Ok(UsageSummary {
            range,
            calls: calls as u32,
            failed: failed as u32,
            prompt_tokens: prompt as u32,
            completion_tokens: completion as u32,
            cost_micro: cost,
            by_model,
            by_purpose,
            by_day,
        })
    }

    /// Ausgaben einer Verbindung seit Monatsbeginn (Mikro-Dollar).
    pub fn spent_this_month(&self, connection_id: &str) -> Result<i64> {
        self.spent_since(connection_id, UsageRange::Month.since(Local::now()))
    }

    fn spent_since(&self, connection_id: &str, since: i64) -> Result<i64> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("ledger lock"))?;
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(cost_micro), 0) FROM usage_event WHERE connection_id = ?1 AND ts >= ?2",
            params![connection_id, since],
            |r| r.get(0),
        )?)
    }

    pub fn budget_state(&self, connection: &LlmConnection) -> Result<BudgetState> {
        let spent = self.spent_this_month(&connection.id)?;
        Ok(budget_state_for(connection, spent))
    }

    pub fn clear(&self) -> Result<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|_| anyhow::anyhow!("ledger lock"))?;
        conn.execute("DELETE FROM usage_event", [])?;
        Ok(())
    }
}

pub fn budget_state_for(connection: &LlmConnection, spent_micro: i64) -> BudgetState {
    let limit_micro = connection
        .monthly_budget_usd
        .filter(|b| *b > 0.0)
        .map(usd_to_micro);
    let ratio = limit_micro.map(|l| spent_micro as f64 / l as f64);
    BudgetState {
        connection_id: connection.id.clone(),
        spent_micro,
        limit_micro,
        ratio,
        enforced: connection.budget_enforced,
    }
}

// ------------------------------------------------------------------ Globals --

/// Liefert die aktuellen Einstellungen -- als Funktion statt als `AppHandle`,
/// damit dieses Modul keine Fensterschicht mitzieht: `settings::get_settings`
/// haengt an Tauri-Dialogen, und ein Test-Exe ohne App-Manifest findet deren
/// `TaskDialogIndirect` nicht (STATUS_ENTRYPOINT_NOT_FOUND beim Start).
pub type SettingsSource = Arc<dyn Fn() -> AppSettings + Send + Sync>;

static LEDGER: OnceLock<Arc<UsageLedger>> = OnceLock::new();
static SETTINGS: OnceLock<SettingsSource> = OnceLock::new();

/// Einmal beim Start: `llm_client` hat keinen `AppHandle`, muss aber buchen
/// und die Preise des aktiven Modells kennen.
pub fn install_globals(ledger: Arc<UsageLedger>, settings: SettingsSource) {
    let _ = LEDGER.set(ledger);
    let _ = SETTINGS.set(settings);
}

pub fn ledger() -> Option<Arc<UsageLedger>> {
    LEDGER.get().cloned()
}

/// Bucht einen Aufruf. Laeuft ohne Ledger (Tests, frueher Start) ins Leere
/// und schluckt Fehler -- die Antwort ist wichtiger als die Buchung.
pub fn record_call(
    purpose: Purpose,
    provider: &PostProcessProvider,
    remote_model: &str,
    usage: TokenUsage,
    duration_ms: u32,
    result: Result<(), String>,
) {
    let Some(ledger) = ledger() else {
        return;
    };
    let settings = SETTINGS.get().map(|source| source());
    let active = settings.as_ref().and_then(|s| s.active_llm_model());
    let event = NewUsageEvent::from_call(
        purpose,
        provider,
        remote_model,
        active,
        usage,
        duration_ms,
        result,
    );
    let ledger = ledger.clone();
    // Nicht auf dem Antwortpfad schreiben: SQLite ist schnell, aber der
    // Aufrufer wartet auf Text, nicht auf Buchhaltung.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = ledger.record(event) {
            warn!("Verbrauch nicht gebucht: {e}");
        }
    });
}

/// Prueft vor einem Aufruf, ob das Monatsbudget der Verbindung des aktiven
/// Modells ausgeschoepft ist -- nur, wenn der Nutzer das Budget als hart
/// markiert hat. Ohne Limit, ohne Ledger oder bei Warnstufe: freie Fahrt.
pub fn check_budget(provider: &PostProcessProvider, remote_model: &str) -> Result<(), String> {
    let (Some(ledger), Some(source)) = (ledger(), SETTINGS.get()) else {
        return Ok(());
    };
    let settings = source();
    let Some((connection, model)) = settings.active_llm_model() else {
        return Ok(());
    };
    if connection.kind != provider.id
        || model.remote_id != remote_model
        || !connection.budget_enforced
    {
        return Ok(());
    }
    let state = ledger
        .budget_state(connection)
        .map_err(|e| format!("Budget nicht pruefbar: {e}"))?;
    if state.exceeded() {
        return Err(format!(
            "Monatsbudget von {:.2} USD fuer „{}“ ausgeschoepft ({:.2} USD verbraucht). In den Einstellungen anheben oder die harte Sperre aufheben.",
            micro_to_usd(state.limit_micro.unwrap_or(0)),
            connection.label,
            micro_to_usd(state.spent_micro)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(id: &str) -> PostProcessProvider {
        PostProcessProvider {
            id: id.to_string(),
            label: id.to_uppercase(),
            base_url: format!("https://{id}.example/v1"),
            allow_base_url_edit: false,
            models_endpoint: None,
            supports_structured_output: false,
        }
    }

    fn connection(id: &str, kind: &str, budget: Option<f64>, enforced: bool) -> LlmConnection {
        LlmConnection {
            id: id.to_string(),
            kind: kind.to_string(),
            label: format!("Konto {id}"),
            base_url: String::new(),
            enabled: true,
            monthly_budget_usd: budget,
            budget_enforced: enforced,
        }
    }

    fn model(conn: &str, remote: &str, pin: f64, pout: f64) -> LlmModelConfig {
        LlmModelConfig {
            id: LlmModelConfig::make_id(conn, remote),
            connection_id: conn.to_string(),
            remote_id: remote.to_string(),
            label: remote.to_string(),
            enabled: true,
            context_limit: None,
            max_input_tokens: None,
            max_output_tokens: None,
            price_input_per_mtok: Some(pin),
            price_output_per_mtok: Some(pout),
            tags: Vec::new(),
        }
    }

    fn ledger() -> (UsageLedger, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = UsageLedger::open(&dir.path().join("usage.db")).unwrap();
        (ledger, dir)
    }

    #[test]
    fn costs_are_token_times_price_per_million_in_micro_dollars() {
        // 1000 Eingabe-Token à 0,40 USD/MTok = 0,0004 USD = 400 µUSD;
        // 500 Ausgabe-Token à 1,60 USD/MTok = 0,0008 USD = 800 µUSD.
        let usage = TokenUsage {
            prompt_tokens: 1000,
            completion_tokens: 500,
        };
        assert_eq!(cost_micro(usage, Some(0.4), Some(1.6)), 1200);
        // Ohne Preis: null, nicht geraten.
        assert_eq!(cost_micro(usage, None, None), 0);
        assert_eq!(cost_micro(usage, Some(0.4), None), 400);
        assert!((micro_to_usd(1200) - 0.0012).abs() < 1e-12);
    }

    #[test]
    fn prices_are_frozen_per_event_from_the_active_model() {
        let conn = connection("openai-1", "openai", None, false);
        let m = model("openai-1", "gpt-4.1-mini", 0.4, 1.6);
        let usage = TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 10,
        };
        let event = NewUsageEvent::from_call(
            Purpose::Summary,
            &provider("openai"),
            "gpt-4.1-mini",
            Some((&conn, &m)),
            usage,
            120,
            Ok(()),
        );
        assert_eq!(event.connection_id, "openai-1");
        assert_eq!(event.model_id, "openai-1:gpt-4.1-mini");
        assert_eq!(event.price_input_per_mtok, Some(0.4));

        // Passt das aktive Modell nicht zum Aufruf (anderes Modell), wird
        // ohne Preis gebucht -- der Anbieter bleibt erkennbar.
        let other = NewUsageEvent::from_call(
            Purpose::Summary,
            &provider("openai"),
            "gpt-4o",
            Some((&conn, &m)),
            usage,
            120,
            Err("boom".into()),
        );
        assert_eq!(other.connection_id, "openai");
        assert_eq!(other.price_input_per_mtok, None);
        assert!(!other.ok);
        assert_eq!(other.error.as_deref(), Some("boom"));
    }

    #[test]
    fn the_ledger_sums_by_model_purpose_and_day() {
        let (ledger, _dir) = ledger();
        let conn = connection("openai-1", "openai", None, false);
        let m = model("openai-1", "gpt-4.1-mini", 1.0, 2.0);
        let p = provider("openai");
        let now = Utc::now().timestamp();
        let mk = |purpose: Purpose, prompt: u64, completion: u64| {
            NewUsageEvent::from_call(
                purpose,
                &p,
                "gpt-4.1-mini",
                Some((&conn, &m)),
                TokenUsage {
                    prompt_tokens: prompt,
                    completion_tokens: completion,
                },
                50,
                Ok(()),
            )
        };
        ledger
            .record_at(mk(Purpose::Summary, 1000, 100), now)
            .unwrap();
        ledger
            .record_at(mk(Purpose::Summary, 1000, 100), now - 60)
            .unwrap();
        ledger
            .record_at(mk(Purpose::Translation, 500, 500), now - 40 * 86_400)
            .unwrap();

        let all = ledger.summary_since(UsageRange::All, 0).unwrap();
        assert_eq!(all.calls, 3);
        assert_eq!(all.prompt_tokens, 2500);
        assert_eq!(all.completion_tokens, 700);
        // 2×(1000×1 + 100×2) + (500×1 + 500×2) = 2400 + 1500
        assert_eq!(all.cost_micro, 3900);
        assert_eq!(all.by_model.len(), 1);
        assert_eq!(all.by_model[0].label, "gpt-4.1-mini · Konto openai-1");
        let purposes: Vec<_> = all
            .by_purpose
            .iter()
            .map(|b| (b.key.as_str(), b.calls))
            .collect();
        assert_eq!(purposes, vec![("summary", 2), ("translation", 1)]);
        assert_eq!(all.by_day.len(), 2, "zwei Kalendertage");

        let recent = ledger.summary_since(UsageRange::Month, now - 3600).unwrap();
        assert_eq!(recent.calls, 2);
        assert_eq!(recent.cost_micro, 2400);

        let events = ledger.events(10, 0).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].ts, now, "neueste zuerst");
        assert_eq!(events[0].cost_micro, 1200);

        ledger.clear().unwrap();
        assert_eq!(ledger.events(10, 0).unwrap().len(), 0);
    }

    #[test]
    fn budget_state_warns_at_80_percent_and_flags_exceeded() {
        let conn = connection("openai-1", "openai", Some(10.0), true);
        let fine = budget_state_for(&conn, usd_to_micro(5.0));
        assert!(!fine.warning() && !fine.exceeded());
        let warn = budget_state_for(&conn, usd_to_micro(8.5));
        assert!(warn.warning() && !warn.exceeded());
        assert!((warn.ratio.unwrap() - 0.85).abs() < 1e-9);
        let over = budget_state_for(&conn, usd_to_micro(10.0));
        assert!(over.exceeded());
        // Ohne Limit gibt es weder Anteil noch Warnung; ein Limit von 0 ist keins.
        let none = budget_state_for(&connection("x", "openai", None, false), 1_000_000);
        assert_eq!(none.ratio, None);
        assert!(!none.warning());
        let zero = budget_state_for(&connection("x", "openai", Some(0.0), true), 1_000_000);
        assert_eq!(zero.limit_micro, None);
    }

    #[test]
    fn spent_this_month_counts_only_this_connection() {
        let (ledger, _dir) = ledger();
        let a = connection("a", "openai", Some(1.0), true);
        let b = connection("b", "openai", None, false);
        let ma = model("a", "m", 1_000_000.0, 0.0); // 1 USD je Token, damit die Zahlen sprechen
        let mb = model("b", "m", 1_000_000.0, 0.0);
        let p = provider("openai");
        let now = Utc::now().timestamp();
        let usage = TokenUsage {
            prompt_tokens: 1,
            completion_tokens: 0,
        };
        ledger
            .record_at(
                NewUsageEvent::from_call(
                    Purpose::Tagging,
                    &p,
                    "m",
                    Some((&a, &ma)),
                    usage,
                    1,
                    Ok(()),
                ),
                now,
            )
            .unwrap();
        ledger
            .record_at(
                NewUsageEvent::from_call(
                    Purpose::Tagging,
                    &p,
                    "m",
                    Some((&b, &mb)),
                    usage,
                    1,
                    Ok(()),
                ),
                now,
            )
            .unwrap();
        assert_eq!(ledger.spent_since("a", 0).unwrap(), 1_000_000);
        assert_eq!(ledger.spent_since("b", 0).unwrap(), 1_000_000);
        let state = ledger.budget_state(&a).unwrap();
        assert!(state.exceeded(), "1 USD von 1 USD");
    }
}
