use crate::model::{ActorId, Principal, Role, TenantId};
use std::collections::BTreeSet;

/// Datenklassen, aufsteigend nach Schutzbedarf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataClass {
    Public,
    Internal,
    Personal,
    /// Art. 9 DSGVO
    Special,
    Restricted,
}

impl DataClass {
    /// Unbekannte Bezeichnung = hoechste Klasse RESTRICTED (fail-closed, ADR-0002 par.6).
    pub fn parse(s: &str) -> DataClass {
        match s.trim().to_ascii_uppercase().as_str() {
            "PUBLIC" => DataClass::Public,
            "INTERNAL" => DataClass::Internal,
            "PERSONAL" => DataClass::Personal,
            "SPECIAL" => DataClass::Special,
            "RESTRICTED" => DataClass::Restricted,
            _ => DataClass::Restricted,
        }
    }
    /// Unklassifiziert = hoechste Klasse RESTRICTED (ADR-0002 par.6).
    pub fn or_unsafe(c: Option<DataClass>) -> DataClass {
        c.unwrap_or(DataClass::Restricted)
    }
}

/// Strengste Datenklasse zweier Einstufungen nach der SDK-Rangfolge
/// (`PUBLIC < INTERNAL < PERSONAL < SPECIAL < RESTRICTED`): nie abschwaechen.
pub fn strictest(a: DataClass, b: DataClass) -> DataClass {
    a.max(b)
}

/// Strengste Einstufung samt Art.-9-Kategorien: Datenklasse = [`strictest`], Art.-9-Kennzeichen =
/// ODER, Kategorien = Vereinigung (Reihenfolge: erst `a`, dann neue aus `b`). Die Kategorien sind
/// eine eigene Menge neben der Datenklasse und gehen bei einer hoeheren Datenklasse nie verloren.
pub fn strictest_art9<K: PartialEq + Clone>(
    a: (DataClass, bool, &[K]),
    b: (DataClass, bool, &[K]),
) -> (DataClass, bool, Vec<K>) {
    let mut kinds = a.2.to_vec();
    for k in b.2 {
        if !kinds.contains(k) {
            kinds.push(k.clone());
        }
    }
    (strictest(a.0, b.0), a.1 || b.1, kinds)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Read,
    Write,
    UseTool(String),
}

#[derive(Debug, Clone)]
pub struct AccessRequest {
    pub tenant: TenantId,
    pub action: Action,
    /// `None` = unklassifiziert, wird als RESTRICTED (hoechste Klasse) behandelt.
    pub class: Option<DataClass>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Erlaubt erst mit gueltigem ApprovalRecord.
    NeedsApproval,
    Deny(&'static str),
}

pub fn authorize(p: &Principal, req: &AccessRequest) -> Decision {
    if req.tenant != *p.tenant() {
        return Decision::Deny("cross-tenant");
    }
    if DataClass::or_unsafe(req.class) > p.max_class() {
        return Decision::Deny("data-class-above-right");
    }
    match &req.action {
        Action::Read => Decision::Allow,
        Action::Write if p.role() >= Role::Member => Decision::Allow,
        Action::Write => Decision::Deny("role"),
        Action::UseTool(t) if p.role() >= Role::Member && p.has_tool(t) => Decision::Allow,
        Action::UseTool(_) => Decision::Deny("tool-right"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionDenied {
    CrossTenant,
    /// Rolle des Entscheiders liegt unter der geforderten Mindestrolle.
    Role,
    /// Rollentrennung: Entscheider hat den Vorgang angelegt, geaendert oder beantragt.
    SelfApproval,
}

/// Darf `p` eine Freigabeentscheidung treffen? `involved` sind alle Akteure, die den Vorgang
/// angelegt, geaendert oder die Freigabe beantragt haben.
pub fn authorize_decision(
    p: &Principal,
    tenant: Option<&TenantId>,
    min_role: Option<Role>,
    separation_of_duties: bool,
    involved: &[ActorId],
) -> Result<(), DecisionDenied> {
    if tenant.is_some_and(|t| t != p.tenant()) {
        return Err(DecisionDenied::CrossTenant);
    }
    if min_role.is_some_and(|r| p.role() < r) {
        return Err(DecisionDenied::Role);
    }
    if separation_of_duties && involved.contains(p.actor()) {
        return Err(DecisionDenied::SelfApproval);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    Local,
    Cloud { provider: String, region: String },
}

#[derive(Debug, Clone)]
pub struct EgressRequest {
    pub class: Option<DataClass>,
    pub route: Route,
    pub purpose: String,
    /// Text wurde redigiert und verifiziert (Pflicht fuer PERSONAL in die Cloud).
    pub redacted_verified: bool,
}

/// Freigegebene Cloud-Ziele; leer = keine Cloud-Route.
#[derive(Debug, Clone, Default)]
pub struct EgressConfig {
    pub policy_version: u32,
    pub allowed_providers: BTreeSet<String>,
}

const FORBIDDEN_PROVIDERS: &[&str] = &["gemini"];

/// Reine, fail-closed Entscheidung vor jedem Netzaufruf.
pub fn decide_egress(cfg: &EgressConfig, req: &EgressRequest) -> Decision {
    let Route::Cloud { provider, .. } = &req.route else {
        return Decision::Allow;
    };
    let provider = provider.to_ascii_lowercase();
    if FORBIDDEN_PROVIDERS.contains(&provider.as_str())
        || !cfg.allowed_providers.contains(&provider)
    {
        return Decision::Deny("provider-not-allowed");
    }
    match DataClass::or_unsafe(req.class) {
        DataClass::Restricted => Decision::Deny("restricted-never-cloud"),
        DataClass::Special => Decision::NeedsApproval,
        DataClass::Personal if !req.redacted_verified => Decision::Deny("personal-needs-redaction"),
        _ => Decision::Allow,
    }
}
