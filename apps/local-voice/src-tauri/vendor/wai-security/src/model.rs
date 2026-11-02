use crate::policy::DataClass;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TenantId(pub String);

impl TenantId {
    /// Mandantennamen mit fuehrendem `_` sind fuer Systemzwecke reserviert (Altdaten-Quarantaene
    /// `_unassigned`, lokale Platzhalter) und duerfen nie einem Principal zugeordnet werden.
    pub fn is_reserved(name: &str) -> bool {
        name.starts_with('_')
    }

    /// Gueltig: weder leer/blank noch reserviert.
    pub fn is_valid(&self) -> bool {
        !self.0.trim().is_empty() && !Self::is_reserved(&self.0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Member,
    Admin,
}

impl Role {
    /// Parst `viewer` / `member` / `admin` (sonst `None`).
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "member" => Some(Role::Member),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// Vom Server verifizierte Sitzung (z. B. aus OIDC-Token oder lokaler Sitzung).
#[derive(Debug, Clone)]
pub struct ServerSession {
    pub tenant: TenantId,
    pub actor: ActorId,
    pub role: Role,
    pub max_class: DataClass,
    pub tool_rights: BTreeSet<String>,
}

/// Unvertrauenswuerdige Felder aus einer Client-Anfrage.
#[derive(Debug, Clone, Default)]
pub struct ClientClaims {
    pub tenant_id: Option<String>,
    pub actor_id: Option<String>,
    pub role: Option<String>,
}

/// Aufgeloeste Identitaet; nur ueber [`resolve_principal`] erzeugbar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    tenant: TenantId,
    actor: ActorId,
    role: Role,
    max_class: DataClass,
    tool_rights: BTreeSet<String>,
}

impl Principal {
    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }
    pub fn actor(&self) -> &ActorId {
        &self.actor
    }
    pub fn role(&self) -> Role {
        self.role
    }
    pub fn max_class(&self) -> DataClass {
        self.max_class
    }
    pub fn has_tool(&self, tool: &str) -> bool {
        self.tool_rights.contains(tool)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Client behauptet andere Identitaet/Rolle als der Server ermittelt hat.
    ClientFieldRejected(&'static str),
    /// Mandant der Serversitzung ist leer oder reserviert (fuehrendes `_`).
    ReservedTenant,
}

/// Principal kommt nur aus der Serversitzung. Client-Felder werden ignoriert,
/// wenn sie mit der Sitzung uebereinstimmen, und abgelehnt, wenn sie abweichen.
pub fn resolve_principal(
    session: &ServerSession,
    claims: &ClientClaims,
) -> Result<Principal, AuthError> {
    if !session.tenant.is_valid() {
        return Err(AuthError::ReservedTenant);
    }
    if claims
        .tenant_id
        .as_ref()
        .is_some_and(|t| *t != session.tenant.0)
    {
        return Err(AuthError::ClientFieldRejected("tenant_id"));
    }
    if claims
        .actor_id
        .as_ref()
        .is_some_and(|a| *a != session.actor.0)
    {
        return Err(AuthError::ClientFieldRejected("actor_id"));
    }
    if claims.role.is_some() {
        return Err(AuthError::ClientFieldRejected("role"));
    }
    Ok(Principal {
        tenant: session.tenant.clone(),
        actor: session.actor.clone(),
        role: session.role,
        max_class: session.max_class,
        tool_rights: session.tool_rights.clone(),
    })
}
