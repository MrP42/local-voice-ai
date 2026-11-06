//! Vertrag des Sprachdienstes (ADR-0021, Abschnitt 5 und Codex-Auflage 2).
//!
//! Local Voice AI stellt Hören und Sprechen (`SpeechIO` in `@wai/voice`) über eine lokale
//! Verbindung bereit (Windows: Named Pipe, macOS/Linux: Unix-Domain-Socket). Dieses Crate
//! enthält nur den Vertrag, keinen Transport und keine Engines:
//!
//! - [`frame`]: Binärrahmen mit festem Kopf und Längenpräfix, harte Größengrenzen, Decoder für
//!   Teillieferungen.
//! - [`msg`]: Steuernachrichten (JSON in Rahmen der Art `Steuer`), Prüfung aller Eingaben,
//!   Zustandsautomat der Sitzung.
//! - [`beweis`]: Handschlag mit Zufallswert und HMAC, der Schlüssel geht nie über die Leitung.
//! - [`endpunkt`]: Ort der Pipe bzw. des Sockets, gleich abgeleitet auf beiden Seiten.
//! - [`client`]: blockierender Client der App-Seite (Handschlag, Kopplung, Erkennen mit Rückstau).
//! - [`fluss`]: Rückstau über Guthaben je Strom und eine Ausgangswarteschlange, in der
//!   Steuerrahmen immer vor Audio laufen und `Abbrechen` alles überholt.
//!
//! Beide Seiten (Client in der App, Dienst in Local Voice AI) benutzen dieselben Typen; die
//! Sicherheitsregeln für den Transport stehen in `docs/leitfaeden/sprachdienst-protokoll.md`.

pub mod beweis;
pub mod client;
pub mod endpunkt;
pub mod fluss;
pub mod frame;
pub mod kopplung;
pub mod msg;

/// Protokollversion; der Handschlag scheitert bei Abweichung (`version_inkompatibel`).
pub const PROTOKOLL: u16 = 1;

/// Grenzen des Vertrags. Beide Seiten prüfen sie; eine Verletzung beendet die Verbindung.
pub mod grenzen {
    /// Länge des Rahmenkopfs in Bytes.
    pub const KOPF: usize = 12;
    /// Größte Nutzlast eines Steuerrahmens (JSON).
    pub const STEUER_MAX: usize = 16 * 1024;
    /// Größte Nutzlast eines Audiorahmens: 1 s PCM16 mono 16 kHz.
    pub const AUDIO_MAX: usize = 32_000;
    /// Gleichzeitig offene Ströme je Verbindung.
    pub const STROEME_MAX: usize = 4;
    /// Guthaben (Audiorahmen) beim Öffnen eines Eingangsstroms.
    pub const GUTHABEN_START: u32 = 16;
    /// Höchstes Guthaben, das eine Seite auf einmal gewähren darf.
    pub const GUTHABEN_MAX: u32 = 64;
    /// Bytes, die in der Ausgangswarteschlange an Audio höchstens warten dürfen.
    pub const AUDIO_PUFFER_MAX: usize = 256 * 1024;
    /// Höchstdauer eines Diktats bzw. einer Äußerung in Sekunden.
    pub const AEUSSERUNG_MAX_S: u32 = 120;
    /// Ohne Audio und ohne Steuerrahmen endet ein offener Strom nach so vielen Sekunden.
    pub const LEERLAUF_S: u32 = 15;
    /// Bis `hallo` bzw. `koppeln` (und `kopplung_schluessel`) da sind, höchstens so viele Sekunden;
    /// das Warten auf die Bestätigung durch den Nutzer zählt nicht dazu.
    pub const HANDSCHLAG_S: u32 = 15;
    /// Eine angemeldete Verbindung ohne offenen Strom und ohne Rahmen endet nach so vielen Sekunden.
    pub const VERBINDUNG_LEERLAUF_S: u32 = 300;
    /// Längster Text für `synthetisieren` (Zeichen).
    pub const TEXT_MAX: usize = 2_000;
    /// Wortpaare je Korrektursignal.
    pub const PAARE_MAX: usize = 32;
    /// Längstes Wort eines Paares (Zeichen).
    pub const WORT_MAX: usize = 48;
    /// Längster Bereichsname (Bytes).
    pub const BEREICH_MAX: usize = 64;
    /// So lange wartet der Dienst auf die Bestätigung einer Kopplung (Sekunden).
    pub const KOPPELN_BESTAETIGUNG_S: u32 = 120;
    /// Kopplungsanfragen je Client-Kennung und Minute; höchstens eine offene zugleich.
    pub const KOPPELN_JE_MINUTE: u32 = 3;
    /// Längster Zugangsschlüssel (Bytes).
    pub const SCHLUESSEL_MAX: usize = 128;
}
