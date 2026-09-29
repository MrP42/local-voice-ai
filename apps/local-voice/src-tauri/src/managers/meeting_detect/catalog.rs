//! Katalog der Meeting-Apps (M5 F16): ordnet einen Registry-Eintrag
//! (`app_key`) einer Klasse zu und liefert das Etikett fuer den Hinweis.
//!
//! Schluessel: `np:<Pfad mit #>` fuer klassische Programme (Registry-Name
//! unveraendert, `\` steht als `#`), `pkg:<Paketfamilie>` fuer Store-Apps.
//! Zugeordnet wird nur ueber den Dateinamen bzw. die Paketfamilie, nie ueber
//! den Ordner: Teams, Zoom & Co. liegen je nach Installation woanders.

use std::path::PathBuf;

/// Klasse eines Eintrags. `MeetingApp`/`Browser` tragen das Etikett.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppClass {
    MeetingApp(&'static str),
    /// Browser: die Registry kennt nur den Browser, nie den Tab (V5).
    Browser(&'static str),
    /// Wird nie gemeldet (z. B. `msedgewebview2.exe`: nicht zuordenbar, V5).
    Ignored,
    Other,
}

/// Etikett fuer Browser-Aufrufe (Google Meet, Teams im Browser, ...).
pub const BROWSER_LABEL: &str = "Browser-Call (z. B. Google Meet)";

/// Dateiname (kleingeschrieben) eines `np:`-Schluessels, sonst `None`.
pub fn exe_name(app_key: &str) -> Option<String> {
    let path = app_key.strip_prefix("np:")?;
    let name = path.rsplit(['#', '\\', '/']).next().unwrap_or(path);
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

/// Paketfamilie (kleingeschrieben) eines `pkg:`-Schluessels, sonst `None`.
pub fn package_family(app_key: &str) -> Option<String> {
    app_key.strip_prefix("pkg:").map(|f| f.to_ascii_lowercase())
}

/// Ausfuehrbare Datei eines `np:`-Schluessels (`#` zurueck zu `\`).
pub fn exe_path_of(app_key: &str) -> Option<PathBuf> {
    let path = app_key.strip_prefix("np:")?;
    Some(PathBuf::from(path.replace('#', "\\")))
}

/// `np:`-Schluessel zu einem Pfad (Umkehrung von [`exe_path_of`]).
pub fn key_of_path(path: &std::path::Path) -> String {
    format!("np:{}", path.to_string_lossy().replace(['\\', '/'], "#"))
}

/// Ist der Eintrag diese App selbst (alle Builds, auch Test-Executables)?
/// Die eigene EXE steht dauerhaft aktiv in der Registry (V5): ohne den Filter
/// meldete die App sich selbst.
pub fn is_own_app(app_key: &str) -> bool {
    match exe_name(app_key) {
        Some(name) => {
            name == "local-voice-ai.exe"
                || name == "sprechstift.exe"
                || name.starts_with("local_voice_ai_lib")
        }
        None => false,
    }
}

const MEETING_EXES: &[(&str, &str)] = &[
    ("ms-teams.exe", "Microsoft Teams"),
    ("msteams.exe", "Microsoft Teams"),
    ("teams.exe", "Microsoft Teams"),
    ("zoom.exe", "Zoom"),
    ("ciscocollabhost.exe", "Webex"),
    ("atmgr.exe", "Webex"),
    ("webexhost.exe", "Webex"),
    ("webex.exe", "Webex"),
    ("slack.exe", "Slack"),
    ("discord.exe", "Discord"),
    ("whatsapp.exe", "WhatsApp"),
    ("skype.exe", "Skype"),
    ("signal.exe", "Signal"),
];

const MEETING_PACKAGES: &[(&str, &str)] = &[
    ("msteams_", "Microsoft Teams"),
    ("microsoftteams_", "Microsoft Teams"),
    ("zoom", "Zoom"),
    ("webex", "Webex"),
    ("ciscospark", "Webex"),
    ("slack", "Slack"),
    ("discord", "Discord"),
    ("whatsapp", "WhatsApp"),
    ("skypeapp", "Skype"),
    ("microsoft.skype", "Skype"),
    ("whispersystems.signal", "Signal"),
    (".signal_", "Signal"),
    ("signalmessenger", "Signal"),
];

const BROWSER_EXES: &[(&str, &str)] = &[
    ("chrome.exe", "Chrome"),
    ("msedge.exe", "Edge"),
    ("firefox.exe", "Firefox"),
    ("brave.exe", "Brave"),
    ("opera.exe", "Opera"),
];

const IGNORED_EXES: &[&str] = &["msedgewebview2.exe"];

/// Ordnet einen Schluessel einer Klasse zu.
pub fn classify(app_key: &str) -> AppClass {
    if let Some(name) = exe_name(app_key) {
        if IGNORED_EXES.contains(&name.as_str()) {
            return AppClass::Ignored;
        }
        if let Some((_, label)) = MEETING_EXES.iter().find(|(exe, _)| *exe == name) {
            return AppClass::MeetingApp(label);
        }
        if let Some((_, label)) = BROWSER_EXES.iter().find(|(exe, _)| *exe == name) {
            return AppClass::Browser(label);
        }
        return AppClass::Other;
    }
    if let Some(family) = package_family(app_key) {
        if let Some((_, label)) = MEETING_PACKAGES
            .iter()
            .find(|(needle, _)| family.contains(needle))
        {
            return AppClass::MeetingApp(label);
        }
        return AppClass::Other;
    }
    AppClass::Other
}

/// Anzeigename: Katalogetikett, Browser-Sammeletikett oder der Dateiname bzw.
/// die Paketfamilie ohne Herausgeberkennung.
pub fn label_for(app_key: &str) -> String {
    match classify(app_key) {
        AppClass::MeetingApp(label) => label.to_string(),
        AppClass::Browser(_) => BROWSER_LABEL.to_string(),
        AppClass::Ignored | AppClass::Other => {
            if let Some(path) = app_key.strip_prefix("np:") {
                path.rsplit('#').next().unwrap_or(path).to_string()
            } else if let Some(family) = app_key.strip_prefix("pkg:") {
                family.split('_').next().unwrap_or(family).to_string()
            } else {
                app_key.to_string()
            }
        }
    }
}

/// Vorrang, wenn mehrere Apps zugleich aktiv sind: Meeting-App vor Browser vor
/// Rest (kleiner = wichtiger). Ein laufender Termin geht vor (P5b).
pub fn rank(class: AppClass) -> u8 {
    match class {
        AppClass::MeetingApp(_) => 0,
        AppClass::Browser(_) => 1,
        AppClass::Other => 2,
        AppClass::Ignored => 3,
    }
}

/// Art fuer die Oberflaeche: `meeting_app` | `browser` | `other`.
pub fn kind_name(class: AppClass) -> &'static str {
    match class {
        AppClass::MeetingApp(_) => "meeting_app",
        AppClass::Browser(_) => "browser",
        AppClass::Ignored | AppClass::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teams_new_package_and_classic_exe_are_meeting_apps() {
        assert_eq!(
            classify("pkg:MSTeams_8wekyb3d8bbwe"),
            AppClass::MeetingApp("Microsoft Teams")
        );
        assert_eq!(
            classify("np:C:#Users#x#AppData#Local#Microsoft#Teams#current#Teams.exe"),
            AppClass::MeetingApp("Microsoft Teams")
        );
    }

    #[test]
    fn webex_host_and_whatsapp_package_are_recognised() {
        assert_eq!(
            classify("np:C:#Users#x#AppData#Local#CiscoSparkLauncher#CiscoCollabHost.exe"),
            AppClass::MeetingApp("Webex")
        );
        assert_eq!(
            classify("pkg:5319275A.WhatsAppDesktop_cv1g1gvanyjgm"),
            AppClass::MeetingApp("WhatsApp")
        );
    }

    #[test]
    fn browsers_get_the_call_label() {
        let key = "np:C:#Program Files#Google#Chrome#Application#chrome.exe";
        assert!(matches!(classify(key), AppClass::Browser("Chrome")));
        assert_eq!(label_for(key), BROWSER_LABEL);
    }

    #[test]
    fn webview2_is_ignored_and_unknown_apps_are_other() {
        assert_eq!(
            classify("np:C:#Program Files (x86)#Microsoft#EdgeWebView#Application#153.0#msedgewebview2.exe"),
            AppClass::Ignored
        );
        assert_eq!(
            classify("np:C:#Program Files#Python311#python.exe"),
            AppClass::Other
        );
        assert_eq!(
            label_for("np:C:#Program Files#Python311#python.exe"),
            "python.exe"
        );
    }

    #[test]
    fn own_builds_are_recognised_by_file_name() {
        for key in [
            "np:C:#Users#w#AppData#Local#Local Voice AI#local-voice-ai.exe",
            "np:C:#x#target#debug#sprechstift.exe",
            "np:C:#x#target#debug#deps#local_voice_ai_lib-f4e693c8af4f36fd.exe",
        ] {
            assert!(is_own_app(key), "{key}");
        }
        assert!(!is_own_app("np:C:#Program Files#Python311#python.exe"));
        assert!(!is_own_app("pkg:MSTeams_8wekyb3d8bbwe"));
    }

    #[test]
    fn path_and_key_round_trip() {
        let key = "np:C:#Program Files#Python311#python.exe";
        let path = exe_path_of(key).unwrap();
        assert_eq!(
            path,
            PathBuf::from("C:\\Program Files\\Python311\\python.exe")
        );
        assert_eq!(key_of_path(&path), key);
    }
}
