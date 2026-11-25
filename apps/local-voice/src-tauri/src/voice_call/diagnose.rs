//! Eine inhaltsfreie Diagnose je fehlgeschlagenem Turn, auch vor dem Versand.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use once_cell::sync::Lazy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Code {
    CliMissing,
    CliLogin,
    CliLimit,
    Zeitueberschreitung,
    Abgebrochen,
    TorVerworfen,
    SchutzGesperrt,
    NurLokal,
    Einstufung,
    AnbieterFehlt,
    Netz,
    Http(u16),
    Unbekannt,
}

impl Code {
    pub(super) fn text(self) -> String {
        match self {
            Self::CliMissing => "cli_missing",
            Self::CliLogin => "cli_login",
            Self::CliLimit => "cli_limit",
            Self::Zeitueberschreitung => "zeitueberschreitung",
            Self::Abgebrochen => "abgebrochen",
            Self::TorVerworfen => "tor_verworfen",
            Self::SchutzGesperrt => "schutz_gesperrt",
            Self::NurLokal => "nur_lokal",
            Self::Einstufung => "einstufung",
            Self::AnbieterFehlt => "anbieter_fehlt",
            Self::Netz => "netz",
            Self::Http(status) => return format!("http_status={status}"),
            Self::Unbekannt => "unbekannt",
        }.to_string()
    }

    pub(super) fn fehler(self) -> String {
        format!("voiceCall.fehler.{}", self.text())
    }
}

/// Übergangsfassade liefert noch Strings: nur feste Präfixe lesen, nie den Rest weiterreichen.
pub(super) fn code(roh: &str) -> Code {
    let text = roh.strip_prefix("voiceCall.fehler.").unwrap_or(roh);
    let kopf = text.split(':').next().unwrap_or_default();
    match kopf {
        "cli_missing" | "claude_not_native" => Code::CliMissing,
        "cli_login" | "cli_not_logged_in" => Code::CliLogin,
        "cli_limit" | "cli_limit_reached" => Code::CliLimit,
        "zeitueberschreitung" | "cli_timeout" => Code::Zeitueberschreitung,
        "abgebrochen" | "cli_cancelled" => Code::Abgebrochen,
        "tor_verworfen" => Code::TorVerworfen,
        "schutz_gesperrt" | "besprechung_cloud_gesperrt" | "projekt.ki_gesperrt" |
        "projekt.bindung" | "projekt.register" | "projekt.fremdkontext" |
        "projekt.ablage" | "projekt.stand_veraltet" => Code::SchutzGesperrt,
        "nur_lokal" | "projekt.ziel_lokal" | "projekt.ziel_compliance" => Code::NurLokal,
        "einstufung" | "projekt.klasse_ziel" | "projekt.anfrage" => Code::Einstufung,
        "anbieter_fehlt" | "projekt.kein_modell" | "projekt.ziel_fenster" => Code::AnbieterFehlt,
        "netz" => Code::Netz,
        _ if text.starts_with("Zeitlimit überschritten") => Code::Zeitueberschreitung,
        _ if text.starts_with("HTTP request failed:") => Code::Netz,
        _ => {
            let status = text.strip_prefix("API request failed with status ")
                .or_else(|| text.strip_prefix("request_failed: HTTP "))
                .or_else(|| text.strip_prefix("http_status="));
            status.and_then(|s| s.split_whitespace().next())
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|s| (100..=599).contains(s))
                .map(Code::Http).unwrap_or(Code::Unbekannt)
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Weg { VorVersand, Cli, Http, Lokal }

fn zeile(weg: Weg, code: Code, dauer_ms: u128) -> String {
    let weg = match weg {
        Weg::VorVersand => "vor_versand", Weg::Cli => "cli",
        Weg::Http => "http", Weg::Lokal => "lokal",
    };
    format!("voice_call: weg={weg} code={} dauer_ms={dauer_ms}", code.text())
}

struct Lauf { start: Instant, weg: Weg, frist: bool, gemeldet: bool, fertig: bool }
type Geteilt = Arc<Mutex<Lauf>>;
static LAEUFE: Lazy<Mutex<HashMap<String, Geteilt>>> = Lazy::new(Default::default);
static FRISTEN: Lazy<Mutex<super::Vorgemerkt>> = Lazy::new(Default::default);
tokio::task_local! { static LAUF: Geteilt; }

fn gehoert_zu(turn: &str, id: &str) -> bool {
    turn == id || turn.strip_prefix(id).is_some_and(|s| s.starts_with('~'))
}

pub(super) fn versand(provider: &crate::settings::PostProcessProvider) {
    let weg = if crate::managers::llm::cli::Cli::from_base_url(&provider.base_url).is_some() {
        Weg::Cli
    } else if super::receiver_is_local(provider) { Weg::Lokal } else { Weg::Http };
    let _ = LAUF.try_with(|l| l.lock().unwrap_or_else(|e| e.into_inner()).weg = weg);
}

/// Die Oberfläche meldet ihre Frist vor dem Abbruch, auch für die Unterturns des Orchestrators.
pub(super) fn frist(id: &str) {
    // IPC kann den Modellstart überholen. Der vorgezogene Ablauf verhindert dann den Versand.
    FRISTEN.lock().unwrap_or_else(|e| e.into_inner()).merke(id);
    for (key, lauf) in LAEUFE.lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if gehoert_zu(key, id) {
            let mut l = lauf.lock().unwrap_or_else(|e| e.into_inner());
            l.frist = true;
            melden(&mut l, Code::Zeitueberschreitung);
        }
    }
}

fn melden(l: &mut Lauf, code: Code) {
    if l.gemeldet { return; }
    l.gemeldet = true;
    let zeile = zeile(l.weg, code, l.start.elapsed().as_millis());
    if code == Code::Abgebrochen { log::info!("{zeile}"); }
    else { log::warn!("{zeile}"); }
}

pub(super) async fn beobachten(
    id: &str,
    turn: impl std::future::Future<Output = Result<String, String>>,
) -> Result<String, String> {
    let lauf = Arc::new(Mutex::new(Lauf {
        start: Instant::now(), weg: Weg::VorVersand, frist: false, gemeldet: false, fertig: false,
    }));
    {
        let mut laeufe = LAEUFE.lock().unwrap_or_else(|e| e.into_inner());
        // Kurzer Nachlauf: Rust kann schon fertig sein, während die Oberfläche ihre Frist meldet.
        // Nur Metadaten behalten; höchstens 256 fertige Läufe, laufende nie verdrängen.
        laeufe.retain(|_, l| {
            let l = l.lock().unwrap_or_else(|e| e.into_inner());
            !l.fertig || l.start.elapsed() < super::EARLY_TTL
        });
        if laeufe.len() >= 256 {
            let alt = laeufe.iter().filter_map(|(id, l)| {
                let l = l.lock().unwrap_or_else(|e| e.into_inner());
                l.fertig.then(|| (id.clone(), l.start))
            }).min_by_key(|(_, start)| *start).map(|(id, _)| id);
            if let Some(alt) = alt { laeufe.remove(&alt); }
        }
        laeufe.insert(id.to_string(), lauf.clone());
    }
    let abgelaufen = FRISTEN.lock().unwrap_or_else(|e| e.into_inner()).0.iter()
        .any(|(key, seit)| gehoert_zu(id, key) && seit.elapsed() < super::EARLY_TTL);
    if abgelaufen { frist(id); }
    if abgelaufen {
        lauf.lock().unwrap_or_else(|e| e.into_inner()).fertig = true;
        return Err(Code::Zeitueberschreitung.fehler());
    }
    let result = LAUF.scope(lauf.clone(), turn).await;
    let mut l = lauf.lock().unwrap_or_else(|e| e.into_inner());
    l.fertig = true;
    if l.frist { return Err(Code::Zeitueberschreitung.fehler()); }
    match result {
        Err(roh) => {
            let code = code(&roh);
            melden(&mut l, code);
            // Projektcodes sind app-eigen; nur den Code erhalten, nie den begleitenden Text.
            let projekt = roh.split(':').next().unwrap_or_default();
            if matches!(projekt,
                "projekt.bindung" | "projekt.budget" | "projekt.stand_veraltet" |
                "projekt.klasse_ziel" | "projekt.ablage" | "projekt.widerrufen" |
                "projekt.anfrage" | "projekt.ki_gesperrt" | "projekt.ungueltig" |
                "projekt.fremdkontext" | "projekt.register" | "projekt.verspaetet" |
                "projekt.kein_modell" | "projekt.ziel_lokal" | "projekt.ziel_compliance" |
                "projekt.ziel_fenster"
            ) { return Err(format!("{projekt}:")); }
            if code == Code::Abgebrochen { Ok(String::new()) } else { Err(code.fehler()) }
        }
        Ok(text) => {
            if text.is_empty() { melden(&mut l, Code::Abgebrochen); }
            Ok(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbildung_und_logzeile_enthalten_keinen_rohtext() {
        for (roh, erwartet) in [
            ("cli_missing: GEHEIM", Code::CliMissing),
            ("cli_not_logged_in: GEHEIM", Code::CliLogin),
            ("cli_limit_reached: GEHEIM", Code::CliLimit),
            ("cli_timeout: GEHEIM", Code::Zeitueberschreitung),
            ("cli_cancelled: GEHEIM", Code::Abgebrochen),
            ("API request failed with status 407 GEHEIM", Code::Http(407)),
            ("HTTP request failed: GEHEIM", Code::Netz),
            ("voiceCall.fehler.tor_verworfen", Code::TorVerworfen),
            ("voiceCall.fehler.schutz_gesperrt", Code::SchutzGesperrt),
            ("voiceCall.fehler.nur_lokal", Code::NurLokal),
            ("voiceCall.fehler.einstufung", Code::Einstufung),
            ("voiceCall.fehler.anbieter_fehlt", Code::AnbieterFehlt),
            ("GEHEIM", Code::Unbekannt),
        ] {
            assert_eq!(code(roh), erwartet);
            let log = zeile(Weg::Cli, code(roh), 12);
            assert_eq!(log, format!("voice_call: weg=cli code={} dauer_ms=12", erwartet.text()));
            assert!(!log.contains("GEHEIM"));
            assert!(!code(roh).fehler().contains("GEHEIM"));
        }
    }

    #[tokio::test]
    async fn rohfehler_wird_einmal_protokolliert_und_nie_ausgeliefert() {
        crate::testlog::installieren();
        let zeilen = || crate::testlog::zeilen().into_iter()
            .filter(|z| z.contains("voice_call: weg=vor_versand code=http_status=599 "))
            .collect::<Vec<_>>();
        let vorher = zeilen().len();
        let r = beobachten("diagnose-synthetisch-http", async {
            Err("API request failed with status 599 GEHEIM-TEST".into())
        }).await;
        assert_eq!(r, Err(Code::Http(599).fehler()));
        let neu = zeilen();
        assert_eq!(neu.len(), vorher + 1);
        assert!(!neu.last().unwrap().contains("GEHEIM-TEST"));
    }

    #[tokio::test]
    async fn frist_hat_vorrang_vor_dem_abbruch_und_wird_nur_einmal_gemeldet() {
        let r = beobachten("diagnose-synthetisch-frist", async {
            frist("diagnose-synthetisch-frist");
            frist("diagnose-synthetisch-frist");
            LAUF.with(|l| {
                let l = l.lock().unwrap();
                assert!(l.frist && l.gemeldet);
            });
            Ok(String::new())
        }).await;
        assert_eq!(r, Err(Code::Zeitueberschreitung.fehler()));
        assert_eq!(beobachten("diagnose-synthetisch-abbruch", async {
            Err("cli_cancelled: GEHEIM-TEST".into())
        }).await, Ok(String::new()));
    }

    #[tokio::test]
    async fn vorgezogene_frist_verhindert_auch_den_versand_eines_unterturns() {
        frist("diagnose-synthetisch-frueh");
        let r = beobachten("diagnose-synthetisch-frueh~0", async {
            panic!("Ein abgelaufener Turn darf nicht mehr starten")
        }).await;
        assert_eq!(r, Err(Code::Zeitueberschreitung.fehler()));
    }
}
