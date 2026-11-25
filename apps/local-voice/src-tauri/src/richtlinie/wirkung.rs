use super::Richtlinie;
use crate::settings::{AppSettings, KeyboardImplementation};
use std::io::{self, Read, Seek};
use std::path::Path;

#[cfg(any(windows, test))]
#[path = "autostart.rs"]
mod autostart;

#[derive(Clone, Copy)]
pub(crate) enum Vorgang {
    ProgrammDownload,
    Selbstupdate,
    Autostart,
    TastaturHook,
    Prozessabfrage,
    Systemton,
}

/// Einziger Erzeuger der maschinenlesbaren Richtlinienmeldung; keine Nutzerdaten.
pub(crate) fn fehler(vorgang: Vorgang, meldung: &str) -> String {
    let schalter = match vorgang {
        Vorgang::ProgrammDownload => "keine_programm_downloads",
        Vorgang::Selbstupdate => "kein_selbstupdate",
        Vorgang::Autostart => "kein_autostart",
        Vorgang::TastaturHook => "keine_tastatur_hooks",
        Vorgang::Prozessabfrage => "keine_besprechungserkennung",
        Vorgang::Systemton => "kein_systemton",
    };
    format!("richtlinie:{schalter}: {meldung}")
}

pub(crate) fn pruefen(p: &Richtlinie, vorgang: Vorgang) -> Result<(), String> {
    let (gesperrt, meldung) = match vorgang {
        Vorgang::ProgrammDownload => (p.keine_programm_downloads, "Programm-Downloads sind gesperrt."),
        Vorgang::Selbstupdate => (p.kein_selbstupdate, "Updates bitte ueber die Release-Seite beziehen: https://github.com/MrP42/local-voice-ai/releases/latest"),
        Vorgang::Autostart => (p.kein_autostart, "Autostart ist gesperrt."),
        Vorgang::TastaturHook => (p.keine_tastatur_hooks, "Tastatur-Hooks sind gesperrt."),
        Vorgang::Prozessabfrage => (p.keine_besprechungserkennung, "Prozessabfragen sind gesperrt."),
        Vorgang::Systemton => (p.kein_systemton, "Systemton ist gesperrt; bitte Mikrofon verwenden."),
    };
    if gesperrt { Err(fehler(vorgang, meldung)) } else { Ok(()) }
}

pub(crate) fn erlauben(vorgang: Vorgang) -> Result<(), String> {
    pruefen(super::aktuell(), vorgang)
}

/// Eine Datenverweigerung beendet auch Spiegel- und Wiederholungswege sofort.
pub(crate) fn ist_gesperrt(fehler: &anyhow::Error) -> bool {
    fehlermeldung(fehler).is_some()
}

/// Umwickelte Fehler an der Oberflaechengrenze als eigene Variante durchreichen.
pub(crate) fn fehlermeldung(fehler: &anyhow::Error) -> Option<String> {
    fehler.chain().map(|ursache| ursache.to_string())
        .find(|meldung| meldung.starts_with("richtlinie:"))
}

pub(crate) fn autostart_erlaubt(p: &Richtlinie, gewuenscht: bool) -> bool {
    gewuenscht && !p.kein_autostart
}

/// Unter Windows entfernt das Autostart-Plugin nach Namen. Mit Richtlinie ist
/// deshalb nur der pfadgepruefte Weg [`autostart_entfernen`] erlaubt.
pub(crate) fn autostart_nur_eigener(p: &Richtlinie) -> bool {
    cfg!(windows) && p.kein_autostart
}

pub(crate) fn autostart_schreiben_mit<T>(p: &Richtlinie, gewuenscht: bool, schreiben: impl FnOnce(bool) -> T) -> T {
    schreiben(autostart_erlaubt(p, gewuenscht))
}

pub(crate) fn systemton_erlaubt(p: &Richtlinie, gewuenscht: bool) -> bool {
    gewuenscht && !p.kein_systemton
}

pub(crate) fn autostart_entfernen(app: &tauri::AppHandle) -> Result<(), String> {
    autostart_entfernen_mit(super::aktuell(), || {
        #[cfg(windows)]
        {
            use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
            // Derselbe Schluessel und App-Name wie tauri-plugin-autostart/auto-launch.
            // is_enabled() genuegt nicht: der Task-Manager kann einen noch vorhandenen
            // Run-Eintrag deaktiviert haben. NotFound ist bei delete_value normal.
            let key = winreg::RegKey::predef(HKEY_CURRENT_USER)
                .open_subkey_with_flags(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run", KEY_QUERY_VALUE | KEY_SET_VALUE)?;
            autostart::entfernen_wenn_eigen(
                || key.get_raw_value(&app.package_info().name),
                |wert| {
                    let exe = std::env::current_exe()?;
                    let exe = exe.to_str().ok_or(io::ErrorKind::InvalidData)?;
                    autostart::gehoert_zur_exe(&wert.bytes, wert.vtype.clone() as u32, exe,
                        |name| std::env::var(name).ok(),
                        |pfad| {
                            if autostart::lexikalisch(pfad).is_none() { return Ok(None); }
                            match std::fs::canonicalize(pfad) {
                                Ok(pfad) => Ok(pfad.to_str().and_then(autostart::lexikalisch)),
                                // Nicht aufloesbar: nie loeschen (lva#905). Die eigene EXE existiert.
                                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                                Err(e) => Err(e),
                            }
                        })
                },
                || key.delete_value(&app.package_info().name),
            )
        }
        #[cfg(not(windows))]
        {
            use tauri_plugin_autostart::ManagerExt;
            let manager = app.autolaunch();
            if manager.is_enabled().map_err(io::Error::other)? {
                manager.disable().map_err(io::Error::other)?;
            }
            Ok(())
        }
    })
}

fn autostart_entfernen_mit(p: &Richtlinie, entfernen: impl FnOnce() -> io::Result<()>) -> Result<(), String> {
    if !p.kein_autostart { return Ok(()); }
    match entfernen() {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(fehler(Vorgang::Autostart, "Autostart konnte nicht entfernt werden.")),
    }
}

pub(crate) fn programm_download_pruefen(vorhanden: bool) -> Result<(), String> {
    programm_download_pruefen_mit(super::aktuell(), vorhanden)
}

fn programm_download_pruefen_mit(p: &Richtlinie, vorhanden: bool) -> Result<(), String> {
    pruefen(p, Vorgang::ProgrammDownload)?;
    if vorhanden { pruefen(p, Vorgang::Selbstupdate)?; }
    Ok(())
}

pub(crate) fn piper_stimme_pruefen(p: &Richtlinie, laufzeit_vorhanden: bool) -> Result<(), String> {
    if !laufzeit_vorhanden && p.keine_programm_downloads {
        return Err(fehler(Vorgang::ProgrammDownload, "Piper-Laufzeit fehlt; bitte durch die IT bereitstellen lassen."));
    }
    Ok(())
}

/// Auch der gemeinsame Transport verweigert Laufzeiten vor der ersten Anfrage.
pub(crate) fn katalog_download_pruefen_mit(p: &Richtlinie, id: &str) -> Result<(), String> {
    use crate::catalog::{tts_entries, Purpose};
    if matches!(id, "piper-runtime" | "phonon-runtime") || [Purpose::TtsRuntime, Purpose::LlmRuntime, Purpose::PhononRuntime]
        .into_iter().any(|zweck| tts_entries(zweck).iter().any(|e| e.id == id))
    {
        pruefen(p, Vorgang::ProgrammDownload)?;
    }
    Ok(())
}

/// Alte Einstellungen bleiben gespeichert, werden aber nie als Freigabe benutzt.
pub(crate) fn einstellungen_begrenzen(p: &Richtlinie, s: &mut AppSettings) {
    if p.kein_autostart { s.autostart_enabled = false; }
    if p.kein_systemton { s.meeting_capture_system = false; }
    if p.keine_besprechungserkennung {
        s.meeting_detect_mode = crate::managers::meeting_detect::DetectMode::Off;
    }
    if p.keine_tastatur_hooks {
        s.keyboard_implementation = KeyboardImplementation::Tauri;
        s.keyboard_hooks_explicitly_enabled = false;
        s.keyboard_hooks_migration_pending = false;
    }
    if p.keine_programm_downloads { s.cli_download_disabled = true; }
    if p.kein_selbstupdate { s.cli_auto_update = false; }
}

/// Am gemeinsamen Serialisierungsende Rohfelder erhalten, auch deren Abwesenheit.
/// Sonst speichert jeder Lesen-Aendern-Schreiben-Weg die wirksamen Ersatzwerte.
pub(crate) fn rohwerte_beibehalten(
    p: &Richtlinie, alt: Option<&serde_json::Value>, neu: &mut serde_json::Value,
) {
    for (gesperrt, felder) in [
        (p.kein_autostart, &["autostart_enabled"][..]),
        (p.kein_systemton, &["meeting_capture_system"][..]),
        (p.keine_besprechungserkennung, &["meeting_detect_mode"][..]),
        (p.keine_tastatur_hooks, &["keyboard_implementation", "keyboard_hooks_explicitly_enabled", "keyboard_hooks_migration_pending"][..]),
        (p.keine_programm_downloads, &["cli_download_disabled"][..]),
        (p.kein_selbstupdate, &["cli_auto_update"][..]),
    ] {
        if !gesperrt { continue; }
        for &feld in felder {
            if let Some(wert) = alt.and_then(|v| v.get(feld)) {
                neu[feld] = wert.clone();
            } else if let Some(map) = neu.as_object_mut() {
                map.remove(feld);
            }
        }
    }
}

/// Gesperrte Freigaben ablehnen, auch bei einem vollstaendigen Import.
pub(crate) fn einstellungen_pruefen(
    p: &Richtlinie, neu: &AppSettings,
) -> Result<(), String> {
    for (freigabe, vorgang) in [
        (neu.autostart_enabled, Vorgang::Autostart),
        (neu.meeting_capture_system, Vorgang::Systemton),
        (neu.meeting_detect_mode != crate::managers::meeting_detect::DetectMode::Off, Vorgang::Prozessabfrage),
        (neu.keyboard_implementation == KeyboardImplementation::HandyKeys
            || neu.keyboard_hooks_explicitly_enabled || neu.keyboard_hooks_migration_pending, Vorgang::TastaturHook),
        (!neu.cli_download_disabled, Vorgang::ProgrammDownload),
        (neu.cli_auto_update, Vorgang::Selbstupdate),
    ] {
        if freigabe { pruefen(p, vorgang)?; }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) struct PeInDaten;

impl std::fmt::Display for PeInDaten {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&fehler(Vorgang::ProgrammDownload, "Programmdatei in Daten entfernt."))
    }
}
impl std::error::Error for PeInDaten {}

pub(crate) fn pe_pruefen(leser: &mut impl Read) -> io::Result<bool> {
    let mut kopf = [0u8; 64];
    let mut gelesen = 0;
    while gelesen < kopf.len() {
        let n = leser.read(&mut kopf[gelesen..])?;
        if n == 0 { return Ok(false); }
        gelesen += n;
    }
    if &kopf[..2] != b"MZ" { return Ok(false); }
    let offset = u32::from_le_bytes(kopf[60..64].try_into().unwrap()) as u64;
    if offset < 64 { return Ok(false); }
    if io::copy(&mut (&mut *leser).take(offset - 64), &mut io::sink())? != offset - 64 {
        return Ok(false);
    }
    let mut signatur = [0; 4];
    match leser.read_exact(&mut signatur) {
        Ok(()) => Ok(&signatur == b"PE\0\0"),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

fn enthaelt_pe(datei: &mut std::fs::File) -> io::Result<bool> {
    if pe_pruefen(datei)? { return Ok(true); }
    datei.rewind()?;
    let mut magic = [0; 4];
    let n = datei.read(&mut magic)?;
    datei.rewind()?;
    if n == 4 && &magic == b"PK\x03\x04" {
        let mut zip = zip::ZipArchive::new(datei).map_err(io::Error::other)?;
        for i in 0..zip.len() {
            let mut eintrag = zip.by_index(i).map_err(io::Error::other)?;
            if eintrag.is_file() && pe_pruefen(&mut eintrag)? { return Ok(true); }
        }
    } else if n >= 2 && magic[..2] == [0x1f, 0x8b] {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(datei));
        for eintrag in tar.entries()? {
            let mut eintrag = eintrag?;
            if eintrag.header().entry_type().is_file() && pe_pruefen(&mut eintrag)? { return Ok(true); }
        }
    }
    Ok(false)
}

/// Nur die gerade geladenen/entpackten Daten pruefen, nie mitgelieferte Laufzeiten.
pub(crate) fn daten_pruefen(p: &Richtlinie, pfad: &Path) -> Result<(), String> {
    if !p.keine_programm_downloads { return Ok(()); }
    use crate::managers::model::download::anchor::Anchor;
    let ergebnis = (|| {
        let parent = pfad.parent().ok_or(io::ErrorKind::InvalidInput)?;
        let name = pfad.file_name().and_then(|n| n.to_str()).ok_or(io::ErrorKind::InvalidInput)?;
        let anker = Anchor::open_path(parent)?;
        anker.check_data(Some(name), &enthaelt_pe, &mut |_| {})
    })();
    daten_ergebnis(ergebnis)
}

/// Entpackordner anlegen. Ohne Richtlinie wie bisher (vorhandener Ordner ist kein
/// Fehler); mit Richtlinie muss der letzte Ordner frisch sein, Eltern duerfen fehlen.
pub(crate) fn frischer_ordner(p: &Richtlinie, pfad: &Path) -> io::Result<()> {
    if !p.keine_programm_downloads {
        return std::fs::create_dir_all(pfad);
    }
    if let Some(eltern) = pfad.parent() {
        std::fs::create_dir_all(eltern)?;
    }
    std::fs::create_dir(pfad)
}

/// Unmittelbar nach dem Anlegen des frischen Ziel-/Temp-Ordners festhalten,
/// noch bevor entpackt wird. Bestehende Installationen gehoeren nie hierher.
pub(crate) fn daten_verankern(p: &Richtlinie, pfad: &Path) -> Result<Option<crate::managers::model::download::anchor::Anchor>, String> {
    if !p.keine_programm_downloads { return Ok(None); }
    crate::managers::model::download::anchor::Anchor::open_path(pfad).map(Some)
        .map_err(|_| fehler(Vorgang::ProgrammDownload, "Daten konnten nicht verankert werden."))
}

pub(crate) fn daten_ordner_pruefen(anker: Option<&crate::managers::model::download::anchor::Anchor>) -> Result<(), String> {
    let Some(anker) = anker else { return Ok(()); };
    daten_ergebnis(anker.check_data(None, &enthaelt_pe, &mut |_| {}))
}

fn daten_ergebnis(ergebnis: io::Result<bool>) -> Result<(), String> {
    match ergebnis {
        Ok(false) => Ok(()),
        Ok(true) => Err(fehler(Vorgang::ProgrammDownload, "Programmdatei in Daten gefunden und entfernt.")),
        Err(_) => Err(fehler(Vorgang::ProgrammDownload, "Daten konnten nicht sicher geprueft oder bereinigt werden.")),
    }
}

/// Tauri leitet Plugin-Aufrufe nicht durch den Invoke-Handler der App.
/// Updater und Autostart daher unmittelbar vor ihrem eigenen Handler pruefen.
pub(crate) fn plugin<P: tauri::plugin::Plugin<tauri::Wry> + 'static>(inner: P) -> impl tauri::plugin::Plugin<tauri::Wry> {
    Geschuetzt(inner)
}

struct Geschuetzt<P>(P);

impl<P: tauri::plugin::Plugin<tauri::Wry>> tauri::plugin::Plugin<tauri::Wry> for Geschuetzt<P> {
    fn name(&self) -> &'static str { self.0.name() }
    fn initialize(&mut self, app: &tauri::AppHandle, config: serde_json::Value) -> Result<(), Box<dyn std::error::Error>> {
        self.0.initialize(app, config)
    }
    fn initialization_script(&self) -> Option<String> { self.0.initialization_script() }
    fn on_event(&mut self, app: &tauri::AppHandle, event: &tauri::RunEvent) { self.0.on_event(app, event); }
    fn extend_api(&mut self, invoke: tauri::ipc::Invoke<tauri::Wry>) -> bool {
        let befehl = invoke.message.command().to_string();
        plugin_aufrufen(super::aktuell(), self.0.name(), &befehl, invoke,
            |invoke| self.0.extend_api(invoke),
            |invoke, meldung| { invoke.resolver.reject(meldung); true })
    }
}

fn plugin_aufrufen<T>(
    p: &Richtlinie, plugin: &str, befehl: &str, anfrage: T,
    weiter: impl FnOnce(T) -> bool, ablehnen: impl FnOnce(T, String) -> bool,
) -> bool {
    match plugin_pruefen(p, plugin, befehl) {
        Ok(()) => weiter(anfrage),
        Err(meldung) => ablehnen(anfrage, meldung),
    }
}

fn plugin_pruefen(p: &Richtlinie, plugin: &str, befehl: &str) -> Result<(), String> {
    match (plugin, befehl) {
        ("updater", "download" | "install" | "download_and_install") => {
            pruefen(p, Vorgang::Selbstupdate)?;
            pruefen(p, Vorgang::ProgrammDownload)
        }
        ("autostart", "enable") => pruefen(p, Vorgang::Autostart),
        // lva#905: `disable` loescht nach Namen und traefe auch fremde Eintraege.
        ("autostart", "disable") if autostart_nur_eigener(p) => pruefen(p, Vorgang::Autostart),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_startkern_schreibt_nur_den_erlaubten_zustand() {
        for (gesperrt, gewuenscht, erwartet) in [(true, true, false), (false, true, true), (false, false, false)] {
            let p = Richtlinie { kein_autostart: gesperrt, ..Default::default() };
            let mut geschrieben = Vec::new();
            autostart_schreiben_mit(&p, gewuenscht, |an| geschrieben.push(an));
            assert_eq!(geschrieben, vec![erwartet]);
        }
    }

    #[test]
    fn plugin_wrapper_erreicht_handler_nur_mit_freigabe() {
        use std::cell::Cell;
        for (plugin, befehl) in [("updater", "install"), ("updater", "download"),
            ("updater", "download_and_install"), ("autostart", "enable")] {
            for blocked in [true, false] {
                let p = if blocked { Richtlinie::streng() } else { Richtlinie::default() };
                let aufrufe = Cell::new(0);
                assert!(plugin_aufrufen(&p, plugin, befehl, (),
                    |()| { aufrufe.set(aufrufe.get() + 1); true },
                    |(), meldung| { assert!(meldung.starts_with("richtlinie:")); true }));
                assert_eq!(aufrufe.get(), usize::from(!blocked));
            }
        }
    }

    #[test]
    fn signatur_lesefehler_bleibt_fehler() {
        struct SignaturFehler(io::Cursor<Vec<u8>>, io::ErrorKind);
        impl Read for SignaturFehler {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.0.position() == 64 { return Err(self.1.into()); }
                self.0.read(buf)
            }
        }
        let mut kopf = vec![0; 64];
        kopf[..2].copy_from_slice(b"MZ");
        kopf[60..].copy_from_slice(&64u32.to_le_bytes());
        for art in [io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert_eq!(pe_pruefen(&mut SignaturFehler(io::Cursor::new(kopf.clone()), art)).unwrap_err().kind(), art);
        }
        assert!(!pe_pruefen(&mut SignaturFehler(io::Cursor::new(kopf), io::ErrorKind::UnexpectedEof)).unwrap());
    }

    #[test]
    fn plugin_sperrt_installation_aber_nicht_versionsanzeige_oder_entfernen() {
        let p = Richtlinie::streng();
        for befehl in ["download", "install", "download_and_install"] {
            assert!(plugin_pruefen(&p, "updater", befehl).is_err());
            assert!(plugin_pruefen(&Richtlinie::default(), "updater", befehl).is_ok());
        }
        assert!(plugin_pruefen(&p, "updater", "check").is_ok());
        assert!(plugin_pruefen(&p, "autostart", "enable").is_err());
        assert_eq!(plugin_pruefen(&p, "autostart", "disable").is_err(), cfg!(windows));
        assert!(plugin_pruefen(&Richtlinie::default(), "autostart", "disable").is_ok());
        assert_eq!(autostart_nur_eigener(&p), cfg!(windows));
        assert!(!autostart_nur_eigener(&Richtlinie::default()));
        assert!(!autostart_erlaubt(&p, true));
        assert!(autostart_erlaubt(&Richtlinie::default(), true));
        assert!(!systemton_erlaubt(&p, true));
        assert!(systemton_erlaubt(&Richtlinie::default(), true));
    }

    #[test]
    fn programm_ersteinrichtung_und_update_sind_getrennt() {
        let p = Richtlinie { kein_selbstupdate: true, ..Default::default() };
        assert!(programm_download_pruefen_mit(&p, true).is_err());
        assert!(programm_download_pruefen_mit(&p, false).is_ok());
        let p = Richtlinie { keine_programm_downloads: true, ..Default::default() };
        assert!(programm_download_pruefen_mit(&p, false).is_err());
        assert!(programm_download_pruefen_mit(&Richtlinie::default(), true).is_ok());
    }

    #[test]
    fn katalog_sperrt_programme_und_erlaubt_modelle_und_stimmen() {
        use crate::catalog::{tts_entries, Purpose};
        let p = Richtlinie { keine_programm_downloads: true, ..Default::default() };
        for zweck in [Purpose::TtsRuntime, Purpose::LlmRuntime, Purpose::PhononRuntime] {
            for entry in tts_entries(zweck) {
                assert!(katalog_download_pruefen_mit(&p, &entry.id).is_err());
                assert!(katalog_download_pruefen_mit(&Default::default(), &entry.id).is_ok());
            }
        }
        for alias in ["piper-runtime", "phonon-runtime"] {
            assert!(katalog_download_pruefen_mit(&p, alias).is_err());
        }
        for zweck in [Purpose::TtsVoice, Purpose::LlmModel, Purpose::LlmEmbedding,
            Purpose::LlmProjector, Purpose::PhononModel, Purpose::PhononVoice,
            Purpose::Diarization, Purpose::SpeakerEmbedding, Purpose::AblageEmbedding] {
            for entry in tts_entries(zweck) {
                assert!(katalog_download_pruefen_mit(&p, &entry.id).is_ok());
            }
        }
    }

    #[test]
    fn richtlinienfehler_sind_kein_grund_fuer_spiegel_oder_wiederholung() {
        assert!(ist_gesperrt(&anyhow::Error::new(PeInDaten).context("Download")));
        assert!(!ist_gesperrt(&anyhow::anyhow!("synthetischer Netzfehler")));
    }

    #[test]
    fn autostart_entfernen_ist_idempotent_und_schluckt_keine_zugriffsfehler() {
        let p = Richtlinie { kein_autostart: true, ..Default::default() };
        let mut entfernt = false;
        assert!(autostart_entfernen_mit(&p, || { entfernt = true; Ok(()) }).is_ok());
        assert!(entfernt);
        assert!(autostart_entfernen_mit(&Default::default(), || panic!("ohne Richtlinie entfernen")).is_ok());
        assert!(autostart_entfernen_mit(&p, || Err(io::ErrorKind::NotFound.into())).is_ok());
        let mut versuche = 0;
        let mut entfernen = || {
            versuche += 1;
            Err(io::ErrorKind::PermissionDenied.into())
        };
        for _ in 0..2 {
            assert!(autostart_entfernen_mit(&p, &mut entfernen).unwrap_err().starts_with("richtlinie:kein_autostart"));
        }
        assert_eq!(versuche, 2, "Zugriffsfehler bleibt fuer den naechsten Start erkennbar");
    }

    #[test]
    fn schreibweg_lehnt_freigaben_ab_und_erlaubt_begrenzten_stand() {
        let p = Richtlinie::streng();
        let mut s = crate::settings::get_default_settings();
        assert!(einstellungen_pruefen(&p, &s).is_err());
        assert!(einstellungen_pruefen(&Richtlinie::default(), &s).is_ok());
        einstellungen_begrenzen(&p, &mut s);
        assert!(einstellungen_pruefen(&p, &s).is_ok());
        for feld in ["autostart_enabled", "meeting_capture_system", "keyboard_hooks_explicitly_enabled", "cli_auto_update"] {
            let mut wert = serde_json::to_value(&s).unwrap();
            wert[feld] = true.into();
            let neu = serde_json::from_value(wert).unwrap();
            assert!(einstellungen_pruefen(&p, &neu).is_err(), "{feld}");
        }
        let mut neu = s.clone();
        neu.cli_download_disabled = false;
        assert!(einstellungen_pruefen(&p, &neu).is_err());
        let mut neu = s.clone();
        neu.keyboard_implementation = KeyboardImplementation::HandyKeys;
        assert!(einstellungen_pruefen(&p, &neu).is_err());
        let mut neu = s.clone();
        neu.keyboard_hooks_migration_pending = true;
        assert!(einstellungen_pruefen(&p, &neu).is_err());
        let mut neu = s;
        neu.meeting_detect_mode = crate::managers::meeting_detect::DetectMode::MeetingApps;
        assert!(einstellungen_pruefen(&p, &neu).is_err());
    }

    #[test]
    fn lesen_aendern_schreiben_erhaelt_rohwerte_und_fehlende_migrationsfelder() {
        let p = Richtlinie::streng();
        let mut alt = crate::settings::get_default_settings();
        alt.autostart_enabled = true;
        alt.meeting_capture_system = true;
        alt.meeting_detect_mode = crate::managers::meeting_detect::DetectMode::MeetingApps;
        alt.keyboard_implementation = KeyboardImplementation::HandyKeys;
        alt.keyboard_hooks_migration_pending = true;
        alt.cli_download_disabled = false;
        alt.cli_auto_update = true;
        let mut roh = serde_json::to_value(&alt).unwrap();
        roh.as_object_mut().unwrap().remove("keyboard_hooks_explicitly_enabled");
        let mut gelesen = alt;
        einstellungen_begrenzen(&p, &mut gelesen);
        assert!(!gelesen.autostart_enabled);
        gelesen.start_hidden = !gelesen.start_hidden;
        assert!(einstellungen_pruefen(&p, &gelesen).is_ok());
        let mut gespeichert = serde_json::to_value(&gelesen).unwrap();
        rohwerte_beibehalten(&p, Some(&roh), &mut gespeichert);
        assert_eq!(gespeichert["autostart_enabled"], true);
        assert_eq!(gespeichert["meeting_capture_system"], true);
        assert_eq!(gespeichert["meeting_detect_mode"], roh["meeting_detect_mode"]);
        assert_eq!(gespeichert["keyboard_hooks_migration_pending"], true);
        assert_eq!(gespeichert["keyboard_implementation"], roh["keyboard_implementation"]);
        assert_eq!(gespeichert["cli_download_disabled"], false);
        assert_eq!(gespeichert["cli_auto_update"], true);
        assert!(gespeichert.get("keyboard_hooks_explicitly_enabled").is_none());
        assert_ne!(gespeichert["start_hidden"], roh["start_hidden"]);
        let mut wieder: AppSettings = serde_json::from_value(gespeichert).unwrap();
        einstellungen_begrenzen(&Richtlinie::default(), &mut wieder);
        assert!(wieder.autostart_enabled);
        gelesen.autostart_enabled = true;
        assert!(einstellungen_pruefen(&p, &gelesen).unwrap_err().starts_with("richtlinie:kein_autostart"));
        let mut ohne = serde_json::to_value(&gelesen).unwrap();
        rohwerte_beibehalten(&Default::default(), Some(&roh), &mut ohne);
        assert!(ohne.get("keyboard_hooks_explicitly_enabled").is_some());
    }

    #[test]
    fn piper_ohne_laufzeit_meldet_sperre_auch_ohne_katalogeintrag() {
        let p = Richtlinie { keine_programm_downloads: true, ..Default::default() };
        assert!(piper_stimme_pruefen(&p, false).unwrap_err().starts_with("richtlinie:keine_programm_downloads"));
        assert!(piper_stimme_pruefen(&p, true).is_ok());
        assert!(piper_stimme_pruefen(&Default::default(), false).is_ok());
    }

    #[test]
    fn fehlerkette_reicht_nur_die_richtlinienvariante_durch() {
        let text = pruefen(&Richtlinie::streng(), Vorgang::ProgrammDownload).unwrap_err();
        let e = anyhow::Error::msg(text.clone()).context("synthetischer Kontext");
        assert_eq!(fehlermeldung(&e), Some(text));
        assert_eq!(fehlermeldung(&anyhow::anyhow!("synthetischer Netzfehler")), None);
    }

    #[test]
    fn entpackordner_ist_nur_mit_richtlinie_zwingend_frisch() {
        let tmp = tempfile::tempdir().unwrap();
        let pfad = tmp.path().join("fehlt/auch/ziel");
        let p = Richtlinie { keine_programm_downloads: true, ..Default::default() };
        frischer_ordner(&p, &pfad).unwrap();
        assert!(frischer_ordner(&p, &pfad).is_err());
        frischer_ordner(&Richtlinie::default(), &pfad).unwrap();
    }

    #[test]
    fn pe_ohne_exe_endung_auch_im_zip_wird_entfernt() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let mut bytes = vec![0; 72];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..68].copy_from_slice(b"PE\0\0");
        let p = Richtlinie { keine_programm_downloads: true, ..Default::default() };
        let datei = tmp.path().join("modell.bin");
        std::fs::write(&datei, &bytes).unwrap();
        assert!(daten_pruefen(&Richtlinie::default(), &datei).is_ok());
        assert!(datei.exists());
        assert!(daten_pruefen(&p, &datei).is_err());
        assert_eq!(datei.exists(), !cfg!(windows));
        let archiv = tmp.path().join("modell.download");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&archiv).unwrap());
        zip.start_file("stimme.onnx", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(&bytes).unwrap();
        zip.finish().unwrap();
        assert!(daten_pruefen(&p, &archiv).is_err());
        assert_eq!(archiv.exists(), !cfg!(windows));
        std::fs::write(&datei, b"synthetische Modelldaten").unwrap();
        assert!(daten_pruefen(&p, &datei).is_ok());
        assert!(datei.exists());
    }

    #[test]
    fn pe_im_tar_gz_und_im_entpackten_unterordner_wird_entfernt() {
        let tmp = tempfile::tempdir().unwrap();
        let mut bytes = vec![0; 68];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&64u32.to_le_bytes());
        bytes[64..].copy_from_slice(b"PE\0\0");
        let p = Richtlinie { keine_programm_downloads: true, ..Default::default() };
        let archiv = tmp.path().join("modell.download");
        let gzip = flate2::write::GzEncoder::new(std::fs::File::create(&archiv).unwrap(), flate2::Compression::default());
        let mut tar = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append_data(&mut header, "modell/daten.bin", bytes.as_slice()).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        assert!(daten_pruefen(&Default::default(), &archiv).is_ok());
        assert!(daten_pruefen(&p, &archiv).is_err());
        assert_eq!(archiv.exists(), !cfg!(windows));
        let ordner = tmp.path().join("entpackt/modell");
        std::fs::create_dir_all(&ordner).unwrap();
        let programm = ordner.join("daten.bin");
        let daten = ordner.join("stimme.onnx");
        std::fs::write(&programm, bytes).unwrap();
        std::fs::write(&daten, b"synthetische Daten").unwrap();
        let anker = daten_verankern(&p, &tmp.path().join("entpackt")).unwrap();
        assert!(daten_ordner_pruefen(anker.as_ref()).is_err());
        assert_eq!(programm.exists(), !cfg!(windows));
        assert!(daten.exists());
    }
}
