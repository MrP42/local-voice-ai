//! Ziel des Versandtors aus der Konfiguration (lva#365 P4c-1, Entscheidung E3).
//!
//! Die reine Regel steht hier ohne Tauri-Zustand, damit sie testbar ist; `AppZiel` (zustand.rs) liest
//! die Einstellungen und startet bei einem lokalen Ziel den Server.
//!
//! - Das Ziel kommt nie aus der Oberfläche. Eine `modell`-Kennung wählt nur unter den konfigurierten,
//!   aktivierten Modellen (`llm_models`), sonst `projekt.kein_modell`.
//! - `lokal` gilt nur für den eingebauten Server (`managers::llm::is_local`). Ein Endpunkt auf
//!   `localhost` (Ollama, Proxy, Relay) zählt nicht, weil er weiter ins Netz leiten kann (wie
//!   `voice_call::receiver_is_local`, Codex M5). `compliance::is_local` und `usage::provider_is_local`
//!   zählen Loopback mit und gelten hier ausdrücklich nicht.
//! - Cloud nimmt höchstens `Internal`. `Confidential` nur, wenn das Regelwerk unter dem Profil EU mit
//!   bekannten Anbieterangaben `Allowed` meldet; dann greifen die Ausnahmen je Ablage-Ordner.
//!   `Personal` und höher gibt es nur lokal.
//! - Meldet das Regelwerk `Blocked`, gibt es kein Ziel (`projekt.ziel_compliance`).
//! - Das Fenster ist das Kleinste aus `max_input_tokens`, `context_limit` und (lokal) dem Kontext des
//!   Servers; kennt es nichts davon, gilt [`FENSTER_VORGABE`].

use super::tor::{Abbruch, Ziel, AUSGABE_VORGABE, FENSTER_VORGABE};
use crate::ablage::ki::ZielAnbieter;
use crate::ablage::zusammenfuehren::Klasse as AblageKlasse;
use crate::managers::compliance::{self, ComplianceProfile, Verdict};
use crate::managers::meetings::llm_call::resolve_provider;
use crate::settings::AppSettings;

pub const CODE_KEIN_MODELL: &str = "projekt.kein_modell";
pub const CODE_COMPLIANCE: &str = "projekt.ziel_compliance";
pub const CODE_ZIEL_LOKAL: &str = "projekt.ziel_lokal";

/// Bestimmt das Ziel aus den Einstellungen. `heute` als `YYYY-MM-DD` (Alter der Anbieterangaben).
/// `server_kontext`: Kontext des lokalen Servers in Token, falls bekannt; er begrenzt nur das Fenster
/// eines lokalen Ziels. `pinned_local` setzt der Aufrufer nach dem Start des Servers.
pub fn aus_einstellungen(
    einstellungen: &AppSettings,
    funktion: &str,
    modell: Option<&str>,
    heute: &str,
    server_kontext: Option<u32>,
) -> Result<Ziel, Abbruch> {
    let kein_modell = Abbruch::ZielUnklar {
        code: CODE_KEIN_MODELL,
    };
    let mut s = einstellungen.for_function(funktion);
    if let Some(id) = modell {
        if !s.llm_model_selectable(id) {
            return Err(kein_modell.clone());
        }
        s.llm_active_model_id = Some(id.to_string());
        s.sync_legacy_from_llm();
    }
    let (provider, name, schluessel) = resolve_provider(&s).map_err(|_| kein_modell)?;
    let lokal = crate::managers::llm::is_local(&provider);
    let bewertung = compliance::assess(
        s.compliance_profile,
        &provider,
        compliance::training_opt_out_for(&s, &provider),
        heute,
    );
    if bewertung.verdict == Verdict::Blocked {
        return Err(Abbruch::ZielUnklar {
            code: CODE_COMPLIANCE,
        });
    }
    let datenklasse_max = if lokal {
        AblageKlasse::Personal
    } else if s.compliance_profile == ComplianceProfile::Eu
        && bewertung.cloud
        && bewertung.verdict == Verdict::Allowed
        && bewertung.reasons.iter().any(|r| r == "eu_ok")
    {
        AblageKlasse::Confidential
    } else {
        AblageKlasse::Internal
    };
    let (kontext, eingabe, ausgabe) = s
        .active_llm_model()
        .map(|(_, m)| (m.context_limit, m.max_input_tokens, m.max_output_tokens))
        .unwrap_or_default();
    let server = server_kontext.filter(|_| lokal);
    let fenster = [kontext, eingabe, server]
        .into_iter()
        .flatten()
        .filter(|n| *n > 0)
        .min()
        .unwrap_or(FENSTER_VORGABE);
    let ausgabe = ausgabe.filter(|n| *n > 0).unwrap_or(AUSGABE_VORGABE);
    Ok(Ziel {
        anbieter: ZielAnbieter {
            id: provider.id.clone(),
            lokal,
            datenklasse_max,
        },
        provider,
        modell: name,
        schluessel: Some(schluessel).filter(|k| !k.is_empty()),
        pinned_local: None,
        fenster,
        ausgabe,
    })
}
