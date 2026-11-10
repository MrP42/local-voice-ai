//! Geraeteseitiger Sync-Kern fuer ADR-0028 Stufe A1 (E3 Geraeteuebergreifende Fortsetzung).
//!
//! Ohne Netz und ohne Systemuhr; der Hub steckt hinter [`hub::HubSpeicher`], die Uhr kommt als
//! Unix-Sekunden vom Aufrufer.
//!
//! - [`umschlag`]: AEAD-Umschlag mit kanonisch gebundenen Steuerfeldern und signiertem Klartext
//!   (E3.0, E3.1, E3.8).
//! - [`identitaet`]: Geraeteschluessel (Ed25519 zum Signieren, X25519 fuer Schluesselpakete).
//! - [`verzeichnis`]: signiertes, hashverkettetes Geraeteverzeichnis (Zulassung, Widerruf, Epoche).
//! - [`paket`]: Schluesselpakete je Empfaengergeraet (Kopplung und Widerruf).
//! - [`kopplung`]: Kopplung an einen Checkpoint des Verzeichnisses mit authentisiertem Rueckkanal.
//! - [`schluessel`]: Epochenschluessel und Kontosalz.
//! - [`speicher`]: Host-Schnittstelle fuer den dauerhaften Geraetezustand.
//! - [`lease`]: Sitzungs-Lease per Vergleichsschreiben, `generation` als Fencing-Wert (E3.3).
//! - [`verlauf`]: Gespraechsbaum ueber `eltern_id`, Zweige, Luecken, Idempotenz (E3.4).
//! - [`grabstein`]: Loeschmarken je `unterhaltung_ref` oder Objekt (E3.2).
//! - [`freigabe`]: Sync-Freigabe, `geraet_lokal`, Datenklasse (E3.5, E3.6, E10).
//! - [`migration`]: wiederholbare Migration bestehender Unterhaltungen (E11).
//! - [`abgleich`]: das Geraet, das alles zusammenfuegt.
//! - [`memhub`]: In-Memory-Hub fuer Tests, auch als boesartiger Hub.
//! - [`testkonto`]: gekoppeltes Testkonto fuer Tests.
//!
//! Keine Inhalte in Fehlern, Berichten, `Debug` oder Logs.

pub mod abgleich;
pub mod fehler;
pub mod freigabe;
pub mod grabstein;
pub mod hub;
pub mod identitaet;
pub mod kennung;
mod kette;
pub mod kopplung;
pub mod lease;
pub mod memhub;
pub mod migration;
pub mod paket;
pub mod schluessel;
pub mod speicher;
pub mod testkonto;
pub mod umschlag;
pub mod verlauf;
pub mod verzeichnis;
mod zeit;

pub use abgleich::{
    Abgleich, Gabelungsbeleg, Herunterladebericht, Hochladebericht, Hoechststand, Kopf,
    MAX_ZURUECKGESTELLT, OffenerWiderruf, Unterhaltung, Weiteres,
};
pub use fehler::SyncFehler;
pub use freigabe::{Einschraenkung, SyncFreigabe, Verbot, hochladen_erlaubt};
pub use grabstein::Grabsteine;
pub use hub::{AuthGeraet, HubFehler, HubSpeicher, LeseFilter, Schreiben};
pub use identitaet::{Ausweis, Geraeteschluessel};
pub use kennung::Hex128;
pub use kopplung::{Kopplung, Kopplungsangebot, Kopplungsantwort, MAX_KOPPLUNG_JSON};
pub use lease::{Gelesen, Halt, LAUFZEIT_S, Lease, LeaseFehler, VERLAENGERUNG_S};
pub use memhub::{Boese, MemHub, Schnappschuss};
pub use migration::{
    AltEintrag, AltUnterhaltung, MIGRATIONSVERSION, Migrationsbericht, Migrationsmarke,
};
pub use paket::{MAX_PAKET_JSON, Schluesselpaket};
pub use schluessel::{Schluessel, Schluesselbund};
pub use speicher::{MemSpeicher, SpeicherFehler, Zustandsspeicher};
pub use testkonto::Testkonto;
pub use umschlag::{
    Geoeffnet, Glied, MAX_KLARTEXT, MAX_UMSCHLAG_JSON, SCHEMA_VERSION, Sammlung, Steuerfelder,
    Umschlag, Ziel, ZielTyp, oeffnen, versiegeln,
};
pub use verlauf::{
    Ansicht, Beitrag, Datenklasse, Einfuegen, Herkunft, Konkurrenz, Luecke, Modus, Uebernahme,
    Verlauf, Zweig, ZweigGrund, Zweigwahl,
};
pub use verzeichnis::{Aenderung, MAX_VERZEICHNISEINTRAG_JSON, Verzeichnis, Verzeichniseintrag};
