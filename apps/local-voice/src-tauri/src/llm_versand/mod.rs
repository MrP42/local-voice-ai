//! Versandvertrag für Modellaufrufe (lva#604, Infrastruktur zu lva#601).
//!
//! Modellaufrufe sollen technisch nur noch über dieses Modul möglich sein. Es prüft frisch und
//! unmittelbar an der Versandgrenze, ob Inhalt an das **konkret aufgelöste** Ziel gehen darf.
//! Jeder Transportversuch braucht eine eigene Prüfung.
//!
//! **Stand: Teilpaket 3 (Kapselung, Einzeltransport, Übergangsfassade).** Der Rohtransport
//! (HTTP, CLI, Bild, Ollama, Modellliste) ist aus `llm_client` und `managers::llm::cli` hierher
//! **verschoben**; `llm_client` hält nur noch Typen und Hilfen ohne Netz. [`versenden`] ist die
//! einzige Versandoperation mit Prüfung. Die bestehenden Aufrufer gehen bis lva#601 über die
//! Übergangsfassade [`uebergang`] (heutige Signaturen, `#[deprecated]`), die **keinen** Schutz
//! des Versandvertrags bietet. Alle Modellpfade stehen in
//! `apps/local-voice/docs/llm-versand-inventar.md`.
//!
//! # Modulbaum und Sichtbarkeit
//!
//! | Modul | Sichtbarkeit | Inhalt |
//! |---|---|---|
//! | [`quellen`] | öffentlich | [`Quellen`]: Herkunft der Inhalte, private Darstellung |
//! | [`ziel`] | öffentlich | [`Ziel`], [`ZielArt`]: aufgelöstes Ziel, Art abgeleitet |
//! | [`pruefer`] | öffentlich | Prüfer, versiegelte [`FreigabeQuelle`] (App-Quelle), `Versandfreigabe` (nur intern) |
//! | [`fehler`] | öffentlich | [`VersandFehler`] mit dem terminalen `Gesperrt` |
//! | [`anfrage`] | öffentlich | [`Anfrage`], [`Inhalt`], [`ChatInhalt`], [`Antwort`] |
//! | `versand` | privat (Reexport) | [`versenden`], [`mit_wiederholung`], Fallbacks, Streamingpuffer, Buchung |
//! | [`uebergang`] | öffentlich, `#[deprecated]` | Fassade mit den heutigen Signaturen, bis lva#601 |
//! | `transport` | **privat** | Rohtransport (HTTP, CLI, Bild, Ollama, Modellliste) |
//!
//! Die Sichtbarkeit setzt der Compiler: in `transport` ist höchstens `pub(super)` erlaubt, es
//! gibt keine Reexporte. `pub(super)` wirkt relativ zum tatsächlichen Modul: in
//! `llm_versand::transport` reicht es bis `llm_versand`, am bisherigen Ort (`llm_client`,
//! `managers::llm::cli`) reichte es bis in die ganze Crate bzw. `managers::llm`. Darum wird der
//! Rohtransport in Teilpaket 3 tatsächlich hierher verschoben.
//!
//! # Vertrag: `versenden`
//!
//! Die einzige Versandoperation hat die Form
//! `versenden(quelle, ziel, quellen, anfrage) -> Result<Antwort, VersandFehler>`. Prüfung und
//! Senden sind gekapselt, nach dem Vorbild `mcp/tools.rs:310–319`. Ablauf, Fallbacks,
//! Streamingpuffer und Buchung stehen in [`versand`](self::versand).
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
//! kein Fallback). Bei Cloud-Zielen mit `Quellen` ungleich `keine()` werden alle
//! Inhaltsausgaben (Deltas, Denktexte, auch CLI-Denktexte) gepuffert und erst nach bestandenem
//! Abschlussvergleich an den Rückruf gegeben; Statusereignisse ohne Inhalt sind erlaubt. Lokale
//! Ziele und `keine()` streamen wie bisher.
//!
//! # Übergang zu lva#601
//!
//! Besprechungspfade werden **nicht** als `keine()` umetikettiert. Die Übergangsfassade
//! [`uebergang`] hat die heutigen Signaturen und das bisherige Verhalten
//! (`#[deprecated(note = "entfällt mit #601")]`); ihre vollständige Aufruferliste (mit und ohne
//! Besprechungsinhalt) ist [`uebergang::AUFRUFER`] und steht im Inventar. lva#601 entfernt die
//! Fassade; erst dann ist der Schutz flächendeckend.
// `versenden`, `mit_wiederholung` und die Reexporte benutzt erst lva#601 (bis dahin geht alles über
// die Übergangsfassade). Mit der Verdrahtung entfällt diese Ausnahme.
#![allow(dead_code, unused_imports)]

pub mod anfrage;
pub mod fehler;
pub mod pruefer;
pub mod quellen;
pub mod uebergang;
pub mod ziel;

#[cfg(test)]
mod testhaken;
#[cfg(test)]
mod testhilfe;

// Privat und ohne Reexport: nur `llm_versand` selbst erreicht den Rohtransport.
mod transport;
mod versand;

pub use anfrage::{Anfrage, Antwort, ChatInhalt, Inhalt, JsonAntwort};
pub use fehler::{VersandFehler, CODE_BESPRECHUNG_CLOUD_GESPERRT};
pub use pruefer::{AppFreigabeQuelle, FreigabeQuelle};
pub use quellen::Quellen;
pub use versand::{mit_wiederholung, versenden};
pub use ziel::{Ziel, ZielArt};
