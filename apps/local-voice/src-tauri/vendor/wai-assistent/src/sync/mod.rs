//! Geraeteseitiger Sync-Kern fuer ADR-0028 Stufe A1 (E3 Geraeteuebergreifende Fortsetzung).
//!
//! Ohne Netz und ohne Systemuhr; der Hub steckt hinter [`hub::HubSpeicher`], die Uhr kommt als
//! Unix-Sekunden vom Aufrufer.
//!
//! - [`umschlag`]: AEAD-Umschlag mit kanonisch gebundenen Steuerfeldern (E3.0, E3.1).
//! - [`schluessel`]: Epochenschluessel und Kontosalz, Widerruf per neuer Epoche.
//! - [`lease`]: Sitzungs-Lease per Vergleichsschreiben, `generation` als Fencing-Wert (E3.3).
//! - [`verlauf`]: Gespraechsbaum ueber `eltern_id`, Zweige, Luecken, Idempotenz (E3.4).
//! - [`grabstein`]: Loeschmarken je `unterhaltung_ref` oder Objekt (E3.2).
//! - [`freigabe`]: Sync-Freigabe, `geraet_lokal`, Datenklasse (E3.5, E3.6, E10).
//! - [`migration`]: wiederholbare Migration bestehender Unterhaltungen (E11).
//! - [`abgleich`]: das Geraet, das alles zusammenfuegt.
//! - [`memhub`]: In-Memory-Hub fuer Tests.
//!
//! Keine Inhalte in Fehlern, Berichten, `Debug` oder Logs.

pub mod abgleich;
pub mod fehler;
pub mod freigabe;
pub mod grabstein;
pub mod hub;
pub mod kennung;
pub mod lease;
pub mod memhub;
pub mod migration;
pub mod schluessel;
pub mod umschlag;
pub mod verlauf;
mod zeit;

pub use abgleich::{
    Abgleich, Herunterladebericht, Hochladebericht, Kopf, MAX_ZURUECKGESTELLT, OffenerWiderruf,
    Unterhaltung, Weiteres, empfaenger_fuer_epoche,
};
pub use fehler::SyncFehler;
pub use freigabe::{Einschraenkung, SyncFreigabe, Verbot, hochladen_erlaubt};
pub use grabstein::Grabsteine;
pub use hub::{AuthGeraet, HubFehler, HubSpeicher, LeseFilter, Schreiben};
pub use kennung::Hex128;
pub use lease::{Gelesen, Halt, LAUFZEIT_S, Lease, LeaseFehler, VERLAENGERUNG_S};
pub use memhub::MemHub;
pub use migration::{
    AltEintrag, AltUnterhaltung, MIGRATIONSVERSION, Migrationsbericht, Migrationsmarke,
};
pub use schluessel::{Schluessel, Schluesselbund};
pub use umschlag::{
    Geoeffnet, SCHEMA_VERSION, Sammlung, Steuerfelder, Umschlag, Ziel, ZielTyp, oeffnen, versiegeln,
};
pub use verlauf::{
    Ansicht, Beitrag, Datenklasse, Einfuegen, Luecke, Modus, Verlauf, Zweig, ZweigGrund,
};
