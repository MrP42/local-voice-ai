//! Microsoft-365-Konto als Integration (Goal „Integrationen“, Issue #66, Paket A5).
//!
//! Ein Konto (Arbeits-, Schul- oder privates Microsoft-Konto) mit drei
//! Faehigkeiten, jede einzeln einschaltbar und hinter dem Freigabe-Tor
//! (Standard fuer Schreibendes: „fragen“, mit Audit):
//!
//! | Faehigkeit        | Was                                    | Scope                         |
//! |-------------------|----------------------------------------|-------------------------------|
//! | `mail.send`       | Mail senden (`POST /me/sendMail`)      | `Mail.Send`                   |
//! | `files.write`     | Datei in OneDrive (PUT / Upload-Sitzung)| `Files.ReadWrite` oder `Files.ReadWrite.AppFolder` |
//! | `calendar.write`  | Notiz an einen Termin                  | `Calendars.ReadWrite`         |
//!
//! Dazu immer `offline_access` (Erneuerungs-Token) und `User.Read` (Name und
//! Adresse des Kontos zur Anzeige). **Scopes nur fuer eingeschaltete Faehigkeiten**:
//! die Anmeldung und jede Erneuerung fragen genau die Scopes an, die
//! `M365Config::required_scopes` liefert. Wird spaeter eine Faehigkeit
//! eingeschaltet, zeigt das Konto „Zustimmung erweitern“ (`needs_consent`), bis man
//! sich erneut anmeldet; ein Aufruf davor wird abgelehnt, ohne Netz.
//!
//! Bausteine:
//! - `config`: Konfiguration und Scopes je Faehigkeit.
//! - `account`: das verschluesselte Konto (Erneuerungs-Token) im Geheimnis-Namensraum.
//! - `service`: Anmeldung (PKCE, Loopback), Zugriffstoken, Anfragen mit 401 -> Erneuern.
//! - `mail`, `drive`, `event`: die drei Faehigkeiten.
//! - `actions`: dieselben Faehigkeiten hinter `gate::run` (Grants, Freigabe, Audit).
//! - `status`: Zustand fuer die Seite Integrationen.
//!
//! Wiederverwendet aus dem Kalender (`calendar::graph`): PKCE, Loopback-Listener,
//! Token-Endpunkt, Drosselung, `GraphState` (nur eine Anmeldung gleichzeitig).
//!
//! # Fehlerfaelle und ihre Absicherung (A5)
//!
//! | Fall | Verhalten | Beleg |
//! |------|-----------|-------|
//! | **Zwei gleichzeitige 401** | EIN Aufruf am Token-Endpunkt (Tor `refresh_gate`), der Zweite nimmt das neue Token | `tests::concurrent_401s_refresh_only_once` |
//! | **Zwei Anmeldungen gleichzeitig** (Doppelklick) | die zweite wird abgewiesen | `tests::a_second_sign_in_is_refused_while_one_runs` |
//! | **Anmeldung abgebrochen / Browser zu / 5 min** | Listener zu, nichts geschrieben | `tests::sign_in_cancelled_*`, `tests::sign_in_without_browser_*` |
//! | **Token ungueltig (invalid_grant)** | totes Token geloescht, Zustand `needs_sign_in`, Meldung | `tests::refresh_invalid_grant_*` |
//! | **401 trotz frischem Token** | `needs_sign_in`, kein dritter Versuch | `tests::a_second_401_*` |
//! | **Faehigkeit nach der Anmeldung eingeschaltet** | `needs_consent`, kein Aufruf | `tests::enabling_a_capability_*` |
//! | **Geheimnis fehlt / kaputt / anderes Konto** | `needs_sign_in`, nie leerer Wert | `tests::missing_or_broken_secret_*` |
//! | **Platte voll beim Schreiben des Tokens** | Anmeldung meldet Fehler, altes Token bleibt; beim Erneuern laeuft der Aufruf mit dem Zugriffstoken weiter | `tests::a_failed_token_write_*` (Namensraum atomar, siehe `calendar::secret`) |
//! | **Netz weg** | `Network`, Mail NICHT als gesendet, Audit `error` | `tests::network_down_*` |
//! | **Verbindung reisst nach dem Senden** | `Uncertain`, kein Wiederholen | `tests::sendmail_connection_lost_is_uncertain` |
//! | **429 / Drosselung** | `Throttled` mit Minuten, kein Wiederholen | `tests::throttled_*` |
//! | **403** (Zustimmung fehlt, Mandant sperrt, nicht Organisator) | `Denied` mit Klartext | `tests::forbidden_*` |
//! | **OneDrive voll** (507) | `StorageFull`, Sitzung abgebrochen | `tests::upload_507_*` |
//! | **Upload bricht mitten ab** | Stueck wiederholt (Stand per `nextExpectedRanges`), danach Sitzung geloescht | `tests::upload_session_*` |
//! | **Abbruch durch den Nutzer** | Sitzung geloescht, `Cancelled` | `tests::upload_cancel_*` |
//! | **Datei aendert sich waehrend des Hochladens** | `Invalid`, Sitzung geloescht | `tests::upload_file_changed_*` |
//! | **Fremde `uploadUrl`** (kein https, Zugangsdaten) | abgelehnt; dorthin geht nie ein Token | `tests::upload_url_*` |
//! | **Umleitung** (302 auf fremden Host) | nie verfolgt | `tests::redirects_are_never_followed` |
//! | **Agent ohne Freigabe** | `Pending`, keine Anfrage am Server | `tests::gate_*` |
//! | **Kindprozess** | keiner (HTTP im Prozess): entfaellt | - |
//!
//! Speicher: Antworten sind auf 8 MiB begrenzt, ein Upload haelt hoechstens ein
//! Stueck (3,125 MiB) im Arbeitsspeicher, die Datei wird von der Platte gelesen; der
//! Mailtext ist auf 1 MiB begrenzt. Bei knappem RAM ist die Last klein und
//! gleichbleibend (ein Stueck); `process_guard::check_ram_for_start` prueft vorher.
//! Kein Audio-Pfad, keine Kindprozesse.

pub mod account;
pub mod actions;
pub mod config;
pub mod drive;
pub mod error;
pub mod event;
pub mod mail;
pub mod service;
pub mod status;

pub use config::{FilesMode, M365Config};
pub use error::M365Error;
pub use service::{Acct, M365Service};
pub use status::M365Status;

#[cfg(test)]
pub(crate) mod test_server;

#[cfg(test)]
mod tests;
