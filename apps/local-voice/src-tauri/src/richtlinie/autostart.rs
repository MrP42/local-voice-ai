use std::io;

/// Lesen und Loeschen sind in der Registry nicht atomar. Unmittelbar vor dem
/// Loeschen deshalb Typ UND Bytes erneut vergleichen; keine Pfade protokollieren.
pub(super) fn entfernen_wenn_eigen<T: PartialEq>(
    mut lesen: impl FnMut() -> io::Result<T>,
    eigen: impl FnOnce(&T) -> io::Result<bool>,
    loeschen: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let alt = lesen()?;
    if eigen(&alt)? && lesen()? == alt { loeschen()?; }
    Ok(())
}

/// Reine Entscheidung: Umgebung und kanonische Pfade liefert der Aufrufer.
/// Nur REG_SZ (1) und REG_EXPAND_SZ (2); kaputtes UTF-16 bleibt ein Lesefehler.
pub(super) fn gehoert_zur_exe(
    bytes: &[u8], typ: u32, exe: &str,
    mut umgebung: impl FnMut(&str) -> Option<String>,
    mut normalisieren: impl FnMut(&str) -> io::Result<Option<String>>,
) -> io::Result<bool> {
    if typ != 1 && typ != 2 { return Ok(false); }
    if bytes.len() % 2 != 0 { return Err(io::ErrorKind::InvalidData.into()); }
    let mut worte: Vec<u16> = bytes.chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
    if worte.last() == Some(&0) { worte.pop(); }
    let mut befehl = String::from_utf16(&worte).map_err(|_| io::ErrorKind::InvalidData)?;
    if befehl.contains('\0') { return Ok(false); }
    if typ == 2 {
        let mut rest = befehl.as_str();
        let mut erweitert = String::new();
        while let Some(i) = rest.find('%') {
            erweitert.push_str(&rest[..i]);
            rest = &rest[i + 1..];
            let Some(ende) = rest.find('%') else { return Ok(false); };
            let Some(wert) = umgebung(&rest[..ende]) else { return Ok(false); };
            // Keine rekursive/teilweise Expansion oder eingeschleuste Befehlsgrenze.
            if wert.contains(['%', '"', '\0']) { return Ok(false); }
            erweitert.push_str(&wert);
            rest = &rest[ende + 1..];
        }
        erweitert.push_str(rest);
        befehl = erweitert;
    }
    let Some(pfad) = programmpfad(&befehl) else { return Ok(false); };
    let Some(pfad) = normalisieren(pfad)? else { return Ok(false); };
    let Some(exe) = normalisieren(exe)? else { return Ok(false); };
    Ok(pfad == exe)
}

fn programmpfad(befehl: &str) -> Option<&str> {
    let befehl = befehl.trim();
    if let Some(rest) = befehl.strip_prefix('"') {
        let ende = rest.find('"')?;
        let danach = &rest[ende + 1..];
        if !danach.is_empty() && !danach.starts_with(char::is_whitespace) { return None; }
        return Some(&rest[..ende]);
    }
    // auto-launch 0.5.0: format!("{} {}", app_path, args.join(" ")),
    // auch bei Leerzeichen im Pfad ohne Anfuehrungszeichen. Die erste EXE-Grenze
    // verwenden, nie einen eigenen Pfad erst in den Argumenten suchen.
    for (i, teil) in befehl.as_bytes().windows(4).enumerate() {
        if teil.eq_ignore_ascii_case(b".exe") {
            let ende = i + 4;
            let danach = &befehl[ende..];
            if danach.is_empty() || danach.starts_with(char::is_whitespace) {
                return Some(&befehl[..ende]);
            }
        }
    }
    None
}

/// Fallback nur fuer fehlende Ziele. Relative Pfade und Geraetenamen ablehnen.
pub(super) fn lexikalisch(pfad: &str) -> Option<String> {
    let pfad = pfad.replace('/', "\\").to_lowercase();
    let pfad = if let Some(rest) = pfad.strip_prefix(r"\\?\unc\") {
        format!(r"\\{rest}")
    } else {
        pfad.strip_prefix(r"\\?\").unwrap_or(&pfad).to_string()
    };
    let (wurzel, rest, minimum) = if let Some(rest) = pfad.strip_prefix(r"\\") {
        (r"\\".to_string(), rest, 2)
    } else if pfad.as_bytes().get(1..3) == Some(b":\\")
        && pfad.as_bytes()[0].is_ascii_alphabetic() {
        (pfad[..3].to_string(), &pfad[3..], 0)
    } else { return None; };
    let mut teile = Vec::new();
    for teil in rest.split('\\') {
        match teil {
            "" | "." => {},
            ".." => { if teile.len() <= minimum { return None; } teile.pop(); },
            _ => {
                if teil.contains([':', '"', '\0', '*', '?']) || teil.ends_with(['.', ' ']) { return None; }
                teile.push(teil);
            }
        }
    }
    if teile.len() <= minimum { return None; }
    Some(wurzel + &teile.join("\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Firma Paket\local-voice-ai.exe";

    fn roh(text: &str) -> Vec<u8> {
        text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
    }

    fn eigen(text: &str, typ: u32) -> bool {
        gehoert_zur_exe(&roh(text), typ, EXE,
            |name| (name == "PAKET").then(|| r"C:\Firma Paket".to_string()),
            |pfad| Ok(lexikalisch(pfad))).unwrap()
    }

    #[test]
    fn eigene_exe_mit_argumenten_und_windows_schreibweisen() {
        for text in [EXE, &format!("{EXE} "), &format!("{EXE} --hidden"),
            &format!("\"{EXE}\""), &format!("\"{EXE}\" --hidden"),
            r"\\?\C:\Firma Paket\local-voice-ai.exe --hidden",
            r"c:\FIRMA PAKET\.\alt\..\LOCAL-VOICE-AI.EXE",
            r"C:/Firma Paket/local-voice-ai.exe"] {
            assert!(eigen(text, 1));
        }
        assert!(eigen(r"%PAKET%\local-voice-ai.exe --hidden", 2));
        assert!(eigen(r#""%PAKET%\local-voice-ai.exe" --hidden"#, 2));
        assert_eq!(lexikalisch(r"\\?\UNC\Server\Share\a.exe"), lexikalisch(r"\\server\share\a.exe"));
    }

    #[test]
    fn fremde_und_unbestimmbare_werte_bleiben_erhalten() {
        for text in ["", " ", r"C:\Installation\local-voice-ai.exe",
            r"C:\fremd.exe --exe C:\Firma Paket\local-voice-ai.exe",
            r"C:\Firma Paket\local-voice-ai.exe.fremd.exe",
            r#""C:\Firma Paket\local-voice-ai.exe"fremd"#,
            r#""C:\Firma Paket\local-voice-ai.exe"#,
            r"local-voice-ai.exe", r"C:local-voice-ai.exe",
            r"%FEHLT%\local-voice-ai.exe", r"%PAKET\local-voice-ai.exe"] {
            assert!(!eigen(text, 1));
            assert!(!eigen(text, 2));
        }
        for typ in [0, 3, 4, 7] { assert!(!eigen(EXE, typ)); }
        assert!(!eigen(r"%PAKET%\local-voice-ai.exe", 1));
        for wert in ["%FEHLT%", "\"C:\\Firma Paket\"", ""] {
            assert!(!gehoert_zur_exe(&roh(r"%PAKET%\local-voice-ai.exe"), 2, EXE,
                |_| Some(wert.into()), |p| Ok(lexikalisch(p))).unwrap());
        }
        assert!(gehoert_zur_exe(&[0], 1, EXE, |_| None, |p| Ok(lexikalisch(p))).is_err());
        assert!(gehoert_zur_exe(&[0, 0xd8], 1, EXE, |_| None, |p| Ok(lexikalisch(p))).is_err());
        assert!(!eigen(&format!("{EXE}\0 --hidden"), 1));
    }

    #[test]
    fn kanonische_pfade_haben_vorrang_und_lesefehler_bleiben_fehler() {
        assert!(gehoert_zur_exe(&roh(r"C:\Alias\app.exe"), 1, EXE, |_| None,
            |_| Ok(Some("kanonisch".into()))).unwrap());
        assert!(!gehoert_zur_exe(&roh(EXE), 1, EXE, |_| None,
            |_| Ok(None)).unwrap());
        assert!(gehoert_zur_exe(&roh(EXE), 1, EXE, |_| None,
            |_| Err(io::ErrorKind::PermissionDenied.into())).is_err());
    }

    #[test]
    fn erneutes_lesen_verhindert_loeschen_eines_ersetzten_wertes() {
        // Tupel bildet Registry-Typ und Rohbytes ab, keine Registry-Zugriffe.
        for (eigen, neu, erwartet) in [(true, (1, 10), true),
            (false, (1, 10), false), (true, (1, 11), false), (true, (2, 10), false)] {
            let mut werte = [(1, 10), neu].into_iter();
            let mut geloescht = false;
            entfernen_wenn_eigen(|| Ok(werte.next().unwrap()), |_| Ok(eigen),
                || { geloescht = true; Ok(()) }).unwrap();
            assert_eq!(geloescht, erwartet);
        }
        let mut aufrufe = 0;
        assert!(entfernen_wenn_eigen(|| {
            aufrufe += 1;
            if aufrufe == 1 { Ok(1) } else { Err(io::ErrorKind::PermissionDenied.into()) }
        }, |_| Ok(true), || panic!("Loeschen nach Lesefehler")).is_err());
        assert!(entfernen_wenn_eigen(|| Ok(1), |_| Ok(true),
            || Err(io::ErrorKind::PermissionDenied.into())).is_err());
        for art in [io::ErrorKind::NotFound, io::ErrorKind::PermissionDenied] {
            let ergebnis = entfernen_wenn_eigen(|| Err::<u8, _>(art.into()),
                |_| panic!("Entscheidung nach Lesefehler"), || panic!("Loeschen nach Lesefehler"));
            assert_eq!(ergebnis.unwrap_err().kind(), art);
        }
    }
}
