//! Versandvertrag für Modellaufrufe (lva#604, Infrastruktur zu lva#601).
//!
//! Modellaufrufe sollen technisch nur noch über dieses Modul möglich sein. Es prüft frisch und
//! unmittelbar an der Versandgrenze, ob Inhalt an das **konkret aufgelöste** Ziel gehen darf.
//! Jeder Transportversuch braucht eine eigene Prüfung.
//!
//! **Stand: Teilpaket 1 (rein additiv, kein Verhalten ändert sich).** Das Modul ist eingebunden
//! und übersetzt, wird aber noch von keiner Stelle benutzt. Es enthält den Modulbaum, die
//! Typen ([`Quellen`], [`Ziel`], [`VersandFehler`]), die Bindung der einmaligen Freigabe an
//! Ziel und Quellen und den Rumpf des Prüfers. Alle Modellpfade stehen in
//! `apps/local-voice/docs/llm-versand-inventar.md`.
//!
//! # Modulbaum und Sichtbarkeit
//!
//! | Modul | Sichtbarkeit | Inhalt |
//! |---|---|---|
//! | [`quellen`] | öffentlich | [`Quellen`]: Herkunft der Inhalte, private Darstellung |
//! | [`ziel`] | öffentlich | [`Ziel`], [`ZielArt`]: aufgelöstes Ziel, Art abgeleitet |
//! | [`pruefer`] | öffentlich | versiegelte [`FreigabeQuelle`], `Versandfreigabe` (nur intern) |
//! | [`fehler`] | öffentlich | [`VersandFehler`] mit dem terminalen `Gesperrt` |
//! | `transport` | **privat** | Rohtransport (HTTP, CLI, Bild), ab Teilpaket 3 |
//!
//! Die Sichtbarkeit setzt der Compiler: in `transport` ist höchstens `pub(super)` erlaubt, es
//! gibt keine Reexporte. `pub(super)` wirkt relativ zum tatsächlichen Modul: in
//! `llm_versand::transport` reicht es bis `llm_versand`, am bisherigen Ort (`llm_client`,
//! `managers::llm::cli`) reichte es bis in die ganze Crate bzw. `managers::llm`. Darum wird der
//! Rohtransport in Teilpaket 3 tatsächlich hierher verschoben.
//!
//! # Vertrag: `versenden`
//!
//! Die einzige Versandoperation (ab Teilpaket 2/3) hat die Form
//! `versenden(quelle, ziel, quellen, anfrage) -> Result<Antwort, VersandFehler>`. Prüfung und
//! Senden sind gekapselt, nach dem Vorbild `mcp/tools.rs:310–319`.
//!
//! Bei einem **Cloud-Ziel** und `Quellen` ungleich `keine()`:
//!
//! 1. **R1:** Richtlinie, Spiegel und Revision frisch lesen (strenger Leseweg, siehe
//!    [`pruefer`]).
//! 2. **Quellenprüfung:** gesperrt bei Kundenprofil „nur lokal“, bei nicht sichtbarer Quelle,
//!    bei `Unbestimmt` und bei jedem Lesefehler.
//! 3. **R2 (Linearisierungspunkt, vor dem Transport):** erneut lesen; weiter nur, wenn alles
//!    gleich R1 ist; sonst neu prüfen, höchstens dreimal, danach `Gesperrt`.
//! 4. Compliance und Budget, Pflicht bei **jedem** Versuch.
//! 5. **Genau ein** Transportversuch an das gebundene Ziel; die Route wird nicht neu aufgelöst.
//! 6. **R3 (Abschlussvergleich):** erneut lesen, **auch bei Transportfehlern, vor Rückgabe
//!    oder Fallback**. R3≠R2: Ergebnis verwerfen, `Gesperrt`, terminal. Der Verbrauch eines
//!    tatsächlich ausgeführten, nachträglich verworfenen Versuchs wird trotzdem gebucht.
//!
//! Bei einem **lokalen Ziel** laufen nur Schritt 4 und 5: `ganz_gesperrt` und `sichtbar()`
//! prüfen unabhängig vom Ziel und dürfen lokale Verarbeitung nicht sperren.
//!
//! **Vorrang:** Sperrt die Besprechungsprüfung, ist das Ergebnis immer
//! `Gesperrt { code: "besprechung_cloud_gesperrt" }`, auch wenn Compliance oder Budget ebenfalls
//! sperren würden. Compliance und Budget bleiben Pflicht, sobald die Besprechungsprüfung
//! bestanden ist.
//!
//! # Grenzen (bewusst, dokumentiert)
//!
//! - **Ein Widerruf nach R3 hält die Ausgabe nicht zurück.** R2 und R3 erkennen nur Änderungen
//!   zwischen zwei Lesungen. Das Freigeben der gepufferten Ausgabe an den Rückruf ist kein
//!   atomarer Vorgang mit Widerrufen. Bereits Versandtes ist nicht rückholbar (ADR-0031).
//! - Ein Widerruf zwischen R2 und dem Transport verhindert den Request nicht mehr; R3 erkennt
//!   ihn, das Ergebnis wird verworfen.
//! - Eine Synchronisation mit den Widerrufsschreibern über die Dauer eines Netzaufrufs ist
//!   **bewusst ausgeschlossen**: sie würde Widerrufe durch langsame Anbieter blockieren.
//! - Ein Cloud-Gateway auf Loopback zählt als lokal (siehe [`ziel`]).
//! - Die nach Terminende aktuelle Teilnehmerlage bleibt die akzeptierte Grenze des Sofortfixes
//!   (`managers/meetings/ki_freigabe.rs:322`).
//!
//! # Fehler- und Streamingvertrag
//!
//! [`VersandFehler`] trägt nie Anbieter- oder Besprechungstext, nur Codes, Zahlen und app-eigene
//! Meldungen; `Gesperrt` ist terminal (nie wiederholen, nie in Chunks zerlegen, nie reparieren,
//! kein Fallback). Bei Cloud-Zielen mit `Quellen` ungleich `keine()` werden ab Teilpaket 3 alle
//! Inhaltsausgaben (Deltas, Denktexte, auch CLI-Denktexte) gepuffert und erst nach bestandenem
//! Abschlussvergleich an den Rückruf gegeben; Statusereignisse ohne Inhalt sind erlaubt. Lokale
//! Ziele und `keine()` streamen wie bisher.
//!
//! # Übergang zu lva#601
//!
//! Besprechungspfade werden **nicht** als `keine()` umetikettiert. Ab Teilpaket 3 gibt es eine
//! Übergangsfassade `uebergang` mit den heutigen Signaturen und dem bisherigen Verhalten
//! (`#[deprecated(note = "entfällt mit #601")]`); ihre vollständige Aufruferliste (mit und ohne
//! Besprechungsinhalt) steht im Inventar. lva#601 entfernt die Fassade; erst dann ist der
//! Schutz flächendeckend.
// Teilpaket 1 bindet das Modul nur ein; benutzt wird es erst ab Teilpaket 2/3 (nichts davon ist
// bis dahin erreichbar, auch die Reexporte nicht). Mit der Verdrahtung (lva#601) entfällt diese
// Ausnahme.
#![allow(dead_code, unused_imports)]

pub mod fehler;
pub mod pruefer;
pub mod quellen;
pub mod ziel;

// Privat und ohne Reexport: nur `llm_versand` selbst erreicht den Rohtransport.
mod transport;

pub use fehler::{VersandFehler, CODE_BESPRECHUNG_CLOUD_GESPERRT};
pub use pruefer::FreigabeQuelle;
pub use quellen::Quellen;
pub use ziel::{Ziel, ZielArt};
