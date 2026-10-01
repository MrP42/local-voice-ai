//! Windows-Mitteilung fuer den Baustein `notify.local` (B4).
//!
//! Die App hatte bisher keine Mitteilung des Betriebssystems (das Hinweisfenster
//! `meeting_prompt` ist ein eigenes Fenster fuer Termine und Einwilligungen). Dieses Modul
//! zeigt eine einfache Windows-Mitteilung ueber `Windows.UI.Notifications`: zwei Textzeilen,
//! keine Schaltflaechen, keine Aktion beim Klick.
//!
//! Die Mitteilung gehoert zur App-Kennung (`identifier` aus `tauri.conf.json`); der
//! Windows-Installer legt dafuer die Startmenue-Verknuepfung mit dieser Kennung an. Ohne sie
//! (Entwicklungsstart) zeigt Windows die Mitteilung gar nicht oder unter fremdem Namen; der
//! Aufruf selbst gelingt trotzdem. Das laesst sich nur im installierten Programm pruefen.
//!
//! Fehlerfaelle: Windows-Mitteilungsdienst aus (Fokus-Assistent zeigt sie nur nicht, der
//! Aufruf gelingt), XML nicht ladbar oder Dienst nicht verfuegbar -> `Err` mit Klartext; es
//! entsteht kein Kindprozess und es wird nichts abgespielt.

/// Der XML-Text der Mitteilung. Titel und Text sind eingesetzt und maskiert.
pub fn toast_xml(title: &str, body: &str) -> String {
    let mut lines = format!("<text>{}</text>", xml_escape(title));
    if !body.trim().is_empty() {
        lines.push_str(&format!("<text>{}</text>", xml_escape(body)));
    }
    format!("<toast><visual><binding template=\"ToastGeneric\">{lines}</binding></visual></toast>")
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Zeigt die Mitteilung. `app_id`: die Kennung der App.
#[cfg(windows)]
pub fn show(app_id: &str, title: &str, body: &str) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    let fail = |what: &str, e: windows::core::Error| format!("{what}: {e}");
    // Der Arbeiter der Engine ist ein eigener Thread ohne COM; schon initialisiert ist harmlos.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let doc = XmlDocument::new().map_err(|e| fail("XML-Dokument", e))?;
    doc.LoadXml(&HSTRING::from(toast_xml(title, body)))
        .map_err(|e| fail("Mitteilungstext", e))?;
    let toast =
        ToastNotification::CreateToastNotification(&doc).map_err(|e| fail("Mitteilung", e))?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(app_id))
        .map_err(|e| fail("Mitteilungsdienst", e))?;
    notifier.Show(&toast).map_err(|e| fail("Anzeigen", e))
}

#[cfg(not(windows))]
pub fn show(_app_id: &str, _title: &str, _body: &str) -> Result<(), String> {
    Err("Mitteilungen des Betriebssystems gibt es nur unter Windows.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toast_text_is_escaped_and_the_body_is_optional() {
        let xml = toast_xml("Müller & Söhne <fertig>", "Zeile \"zwei\"\u{7}");
        assert!(xml.contains("<text>Müller &amp; Söhne &lt;fertig&gt;</text>"), "{xml}");
        assert!(xml.contains("<text>Zeile &quot;zwei&quot; </text>"), "{xml}");
        let only_title = toast_xml("Fertig", "  ");
        assert_eq!(only_title.matches("<text>").count(), 1);
        assert!(only_title.starts_with("<toast>") && only_title.ends_with("</toast>"));
    }

    /// Der XML-Text ist wohlgeformt: Windows laedt ihn (nur unter Windows, ohne etwas zu zeigen).
    #[cfg(windows)]
    #[test]
    fn windows_accepts_the_xml_without_showing_anything() {
        use windows::core::HSTRING;
        use windows::Data::Xml::Dom::XmlDocument;
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let doc = XmlDocument::new().unwrap();
        doc.LoadXml(&HSTRING::from(toast_xml("Größe & Änderung", "Text <1>")))
            .unwrap();
    }
}
