//! Rechte: wer darf was mit welcher Integration (A1, E3).
//!
//! `effective_mode` ist rein (keine Datenbank, keine Uhr) und damit das, was
//! Tests und Sicherheitsreview lesen. Die Regel, von streng nach locker:
//!
//! 1. Integration aus -> `Off`, fuer jeden, auch den Nutzer.
//! 2. Die Art bietet die Faehigkeit nicht an, oder die Richtung der Integration
//!    erlaubt ihre Zugriffsart nicht (lesend/schreibend) -> `Off`.
//! 3. Der Nutzer in der Oberflaeche -> `Allow` (er braucht keine Freigabe).
//! 4. Sonst das gespeicherte Recht, bei fehlender Zeile die Vorgabe
//!    (`default_mode`); bei externen Agenten gilt zusaetzlich das Recht je
//!    Werkzeug (`tool_mode`): das Strengere gewinnt.
//! 5. Nie dauerhaft erlaubt (`Capability::never_allow`, Aufnahme starten):
//!    `Allow` wird zu `Ask`.

use std::collections::HashMap;

use super::model::{Caller, Capability, GrantMode, Integration};

/// Gespeicherte Rechte EINER Integration: (Faehigkeit, Aufrufer) -> Modus.
pub type GrantSet = HashMap<(Capability, Caller), GrantMode>;

/// Vorgabe, wenn keine Zeile gespeichert ist (E3): externe Agenten alles aus;
/// Workflow und lokaler Agent lesen frei und fragen bei Schreibendem;
/// `media.fetch` ist fuer alle ausser dem Nutzer aus.
pub fn default_mode(cap: Capability, caller: Caller) -> GrantMode {
    // Das YouTube-Dateiholen (externes Werkzeug, Schalter „privat“, E1) ist nie
    // von selbst an: fuer jeden ausser dem Nutzer aus, bis es jemand einschaltet.
    if cap == Capability::MediaFetch && caller != Caller::User {
        return GrantMode::Off;
    }
    match caller {
        Caller::User => GrantMode::Allow,
        Caller::AgentExternal => GrantMode::Off,
        Caller::Workflow | Caller::AgentLocal => match cap.access() {
            super::model::Access::Read => GrantMode::Allow,
            super::model::Access::Write => GrantMode::Ask,
        },
    }
}

/// Warum ein Zugriff `Off` ist (fuer Audit-Detail und Oberflaeche).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OffReason {
    IntegrationDisabled,
    CapabilityNotOffered,
    DirectionBlocks,
    GrantOff,
    ToolOff,
}

impl OffReason {
    pub fn as_str(self) -> &'static str {
        match self {
            OffReason::IntegrationDisabled => "integration_disabled",
            OffReason::CapabilityNotOffered => "capability_not_offered",
            OffReason::DirectionBlocks => "direction_blocks",
            OffReason::GrantOff => "grant_off",
            OffReason::ToolOff => "tool_off",
        }
    }

    /// Klartext fuer Meldungen an Nutzer und Agenten.
    pub fn message(self) -> &'static str {
        match self {
            OffReason::IntegrationDisabled => "Die Integration ist ausgeschaltet.",
            OffReason::CapabilityNotOffered => "Diese Integration bietet die Fähigkeit nicht an.",
            OffReason::DirectionBlocks => {
                "Die Richtung der Integration erlaubt diese Fähigkeit nicht."
            }
            OffReason::GrantOff => "Das Recht für diese Fähigkeit steht auf „aus“.",
            OffReason::ToolOff => "Das Werkzeug ist für diesen Zugang ausgeschaltet.",
        }
    }
}

/// Wirksamer Modus samt Grund, falls `Off`.
pub fn explain(
    i: &Integration,
    cap: Capability,
    caller: Caller,
    grants: &GrantSet,
    tool_mode: Option<GrantMode>,
) -> (GrantMode, Option<OffReason>) {
    if !i.enabled {
        return (GrantMode::Off, Some(OffReason::IntegrationDisabled));
    }
    if !i.kind.capabilities().contains(&cap) {
        return (GrantMode::Off, Some(OffReason::CapabilityNotOffered));
    }
    if !i.direction.permits(cap.access()) {
        return (GrantMode::Off, Some(OffReason::DirectionBlocks));
    }
    if caller == Caller::User {
        return (GrantMode::Allow, None);
    }
    let stored = grants
        .get(&(cap, caller))
        .copied()
        .unwrap_or_else(|| default_mode(cap, caller));
    let mut mode = stored;
    let mut reason = (stored == GrantMode::Off).then_some(OffReason::GrantOff);
    if let Some(tool) = tool_mode {
        if tool < mode {
            mode = tool;
            reason = (tool == GrantMode::Off).then_some(OffReason::ToolOff);
        }
    }
    if cap.never_allow() && mode == GrantMode::Allow {
        mode = GrantMode::Ask;
    }
    (mode, reason)
}

/// Wirksamer Modus (siehe Moduldoku).
pub fn effective_mode(
    i: &Integration,
    cap: Capability,
    caller: Caller,
    grants: &GrantSet,
    tool_mode: Option<GrantMode>,
) -> GrantMode {
    explain(i, cap, caller, grants, tool_mode).0
}

#[cfg(test)]
mod tests;
