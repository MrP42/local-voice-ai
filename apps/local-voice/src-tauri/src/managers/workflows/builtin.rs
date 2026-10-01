//! Eingebaute Bausteine, die keine App-Funktion brauchen (B1): `wait`.
//!
//! `wait` blockiert nie: beim ersten Aufruf meldet es `Defer` bis zum Ablauf der
//! Zeit, die Engine parkt den Lauf (`next_run_at`), und beim naechsten Aufruf ist die
//! Frist verstrichen. Die Frist rechnet ab dem ERSTEN Beginn des Schritts
//! (`ctx.step_started_at`), nicht ab dem letzten Aufruf: ein Neustart der App
//! verlaengert das Warten nicht und verkuerzt es nicht.

use serde_json::{json, Value};

use super::action::{
    Action, EffectKind, Needs, NeedsError, RunCtx, StepError, StepOutput,
};

pub struct WaitAction;

fn minutes_of(params: &Value) -> Result<u64, StepError> {
    params
        .get("minutes")
        .and_then(Value::as_u64)
        .filter(|m| (1..=10_080).contains(m))
        .ok_or_else(|| {
            StepError::Permanent("„minutes“ muss eine ganze Zahl von 1 bis 10080 sein.".to_string())
        })
}

impl Action for WaitAction {
    fn id(&self) -> &str {
        "wait"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Pure
    }

    fn needs(&self, _params: &Value) -> Result<Option<Needs>, NeedsError> {
        Ok(None)
    }

    fn describe(&self, params: &Value) -> String {
        match params.get("minutes").and_then(Value::as_u64) {
            Some(m) => format!("{m} Minuten warten"),
            None => "Warten".to_string(),
        }
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let minutes = minutes_of(params)?;
        let deadline = ctx
            .step_started_at
            .saturating_add((minutes as i64).saturating_mul(60_000));
        let now = ctx.now_ms();
        if now >= deadline {
            return Ok(StepOutput::with_data(json!({"waited_minutes": minutes}))
                .summary(&format!("{minutes} Minuten gewartet")));
        }
        Err(StepError::Defer {
            retry_after_ms: (deadline - now) as u64,
            reason: format!("Wartet bis zum Ablauf von {minutes} Minuten."),
        })
    }
}
