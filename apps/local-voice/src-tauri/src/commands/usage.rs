//! Verbrauchsuebersicht: was die Sprachmodelle gekostet haben, je Zeitraum,
//! Modell und Zweck -- und wie voll die Monatsbudgets sind.

use std::sync::Arc;
use tauri::{AppHandle, State};

use crate::managers::usage::{BudgetState, UsageEvent, UsageLedger, UsageRange, UsageSummary};
use crate::settings;

#[tauri::command]
#[specta::specta]
pub async fn usage_summary(
    ledger: State<'_, Arc<UsageLedger>>,
    range: UsageRange,
) -> Result<UsageSummary, String> {
    let ledger = ledger.inner().clone();
    tauri::async_runtime::spawn_blocking(move || ledger.summary(range))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usage_events(
    ledger: State<'_, Arc<UsageLedger>>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<UsageEvent>, String> {
    let ledger = ledger.inner().clone();
    let limit = limit.unwrap_or(100).min(1000);
    let offset = offset.unwrap_or(0);
    tauri::async_runtime::spawn_blocking(move || ledger.events(limit, offset))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn usage_clear(ledger: State<'_, Arc<UsageLedger>>) -> Result<(), String> {
    let ledger = ledger.inner().clone();
    tauri::async_runtime::spawn_blocking(move || ledger.clear())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Budgetstand jeder Verbindung mit gesetztem Limit -- fuer die Fussleiste
/// und die Verbindungsliste. Verbindungen ohne Limit fehlen: nichts zu sagen.
#[tauri::command]
#[specta::specta]
pub async fn usage_budget_states(
    app: AppHandle,
    ledger: State<'_, Arc<UsageLedger>>,
) -> Result<Vec<BudgetState>, String> {
    let ledger = ledger.inner().clone();
    let connections = settings::get_settings(&app).llm_connections;
    tauri::async_runtime::spawn_blocking(move || {
        connections
            .iter()
            .filter(|c| c.monthly_budget_usd.is_some_and(|b| b > 0.0))
            .map(|c| ledger.budget_state(c))
            .collect::<anyhow::Result<Vec<_>>>()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
