//! Windows-Explorer-Kontextmenü „Mit Local Voice AI vorlesen" für Dokumente.
//!
//! Einträge liegen unter HKCU\Software\Classes\SystemFileAssociations\<ext>\
//! shell\LocalVoiceAI.Read — reine Benutzer-Registry, keine Adminrechte.
//! Hinweis: Ein Kontextmenü auf MARKIERTEM TEXT in fremden Anwendungen ist
//! unter Windows systemseitig nicht möglich; dafür existiert der globale
//! Hotkey (Zwischenablage vorlesen).

#![cfg(windows)]

use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

pub const DOCUMENT_EXTENSIONS: [&str; 12] = [
    ".txt", ".md", ".pdf", ".docx", ".png", ".jpg", ".jpeg", ".webp", ".tif", ".tiff", ".bmp",
    ".gif",
];
const VERB: &str = "LocalVoiceAI.Read";
const PROD_PREFIX: &str = "Software\\Classes\\SystemFileAssociations";

fn shell_key_path(prefix: &str, ext: &str) -> String {
    format!("{prefix}\\{ext}\\shell\\{VERB}")
}

pub fn register_under(prefix: &str, exe: &std::path::Path) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let exe_str = exe.to_string_lossy();
    for ext in DOCUMENT_EXTENSIONS {
        let (shell, _) = hkcu
            .create_subkey(shell_key_path(prefix, ext))
            .map_err(|e| format!("registry write failed for {ext}: {e}"))?;
        shell
            .set_value("", &"Mit Local Voice AI vorlesen")
            .map_err(|e| e.to_string())?;
        shell
            .set_value("Icon", &format!("\"{exe_str}\""))
            .map_err(|e| e.to_string())?;
        let (command, _) = shell.create_subkey("command").map_err(|e| e.to_string())?;
        command
            .set_value("", &format!("\"{exe_str}\" --read-file \"%1\""))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn unregister_under(prefix: &str) {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for ext in DOCUMENT_EXTENSIONS {
        let _ = hkcu.delete_subkey_all(shell_key_path(prefix, ext));
    }
}

/// Kontextmenü passend zum Setting herstellen; bei aktivem Eintrag wird der
/// EXE-Pfad aktualisiert (Builds wandern zwischen debug/release).
pub fn sync(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        register_under(PROD_PREFIX, &exe)
    } else {
        unregister_under(PROD_PREFIX);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Läuft gegen einen Testast der Benutzer-Registry, nie gegen die echten
    /// SystemFileAssociations.
    #[test]
    fn register_writes_verb_and_command_and_unregister_cleans_up() {
        let prefix = "Software\\LocalVoiceAI-Test\\SystemFileAssociations";
        let exe = std::path::Path::new(r"C:\Programme\local-voice-ai.exe");
        register_under(prefix, exe).unwrap();

        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let command: String = hkcu
            .open_subkey(format!("{prefix}\\.txt\\shell\\LocalVoiceAI.Read\\command"))
            .unwrap()
            .get_value("")
            .unwrap();
        assert!(command.contains("--read-file"));
        assert!(command.contains("local-voice-ai.exe"));

        unregister_under(prefix);
        assert!(
            hkcu.open_subkey(format!("{prefix}\\.txt\\shell\\LocalVoiceAI.Read"))
                .is_err(),
            "Eintrag muss restlos verschwinden"
        );
        let _ = hkcu.delete_subkey_all("Software\\LocalVoiceAI-Test");
    }
}
