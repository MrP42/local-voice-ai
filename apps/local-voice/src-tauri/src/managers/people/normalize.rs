//! Normalisierung fuer den Personenabgleich (M5, P5d; `entwurf/m5-m6-kalender-export.md`
//! F17). Reine Funktionen ohne Speicher: E-Mail (klein, getrimmt), Name
//! („Berg, Anna“ und „Anna Berg“ sind derselbe Schluessel), Firma aus der
//! Domain (ohne Freemail-Anbieter). Bewusst KEINE unscharfe Zuordnung: eine
//! Fehlzusammenfuehrung waere still, also gilt nur exakte Gleichheit nach dieser
//! Normalisierung.

/// Freemail-Anbieter: ihre Domain sagt nichts ueber eine Firma.
const FREEMAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "gmx.de",
    "gmx.net",
    "gmx.at",
    "gmx.ch",
    "gmx.com",
    "web.de",
    "outlook.com",
    "outlook.de",
    "hotmail.com",
    "hotmail.de",
    "live.com",
    "live.de",
    "msn.com",
    "t-online.de",
    "icloud.com",
    "me.com",
    "mac.com",
    "yahoo.com",
    "yahoo.de",
    "ymail.com",
    "freenet.de",
    "posteo.de",
    "mailbox.org",
    "protonmail.com",
    "proton.me",
    "aol.com",
    "aol.de",
    "1und1.de",
    "online.de",
];

/// Laengster Name, den eine Person tragen darf.
pub const MAX_NAME_CHARS: usize = 120;

/// E-Mail-Adresse in der gespeicherten Form: ohne `mailto:`, getrimmt, klein.
/// `None`, wenn es keine brauchbare Adresse ist (genau ein `@`, Punkt in der
/// Domain, keine Leer- oder Sonderzeichen).
pub fn normalize_email(s: &str) -> Option<String> {
    let mut a = s.trim();
    if a.len() >= 7 && a[..7].eq_ignore_ascii_case("mailto:") {
        a = a[7..].trim();
    }
    let a = a.to_lowercase();
    if a.is_empty() || a.len() > 254 {
        return None;
    }
    if a.chars()
        .any(|c| c.is_whitespace() || c.is_control() || "<>\",;()[]\\".contains(c))
    {
        return None;
    }
    let mut parts = a.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    let ok = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.');
    ok.then_some(a)
}

/// Grundbuchstabe eines Buchstabens mit Diakritikum (Latin-1 und Latin
/// Extended-A, was in deutschen Namen vorkommt). Alles andere bleibt.
fn fold_char(c: char, out: &mut String) {
    let folded = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' => "s",
        'ß' => "ss",
        'ţ' | 'ť' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ý' | 'ÿ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'æ' => "ae",
        'œ' => "oe",
        // Kombinierende Akzente (zerlegte Schreibweise) fallen weg.
        '\u{0300}'..='\u{036f}' => "",
        other => {
            out.push(other);
            return;
        }
    };
    out.push_str(folded);
}

/// Schluessel eines Namens: „Berg, Anna“ -> „anna berg“. Genau ein Komma mit
/// Text auf beiden Seiten dreht die Teile um („Nachname, Vorname“); Diakritika
/// fallen weg, Gross-/Kleinschreibung und Mehrfach-Leerzeichen zaehlen nicht,
/// Anfuehrungszeichen ebenso wenig. Leer, wenn nichts Brauchbares bleibt.
pub fn normalize_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, '"' | '\'' | '`' | '„' | '“' | '”' | '‚' | '‘' | '’'))
        .collect();
    let cleaned = cleaned.trim();
    let swapped;
    let base = match cleaned.split_once(',') {
        Some((last, first))
            if !first.contains(',') && !last.trim().is_empty() && !first.trim().is_empty() =>
        {
            swapped = format!("{} {}", first.trim(), last.trim());
            swapped.as_str()
        }
        _ => cleaned,
    };
    let mut folded = String::with_capacity(base.len());
    for c in base.to_lowercase().chars() {
        fold_char(c, &mut folded);
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Der Name, wie er angezeigt wird: getrimmt, Leerraum zusammengezogen, auf
/// [`MAX_NAME_CHARS`] gekuerzt. „Berg, Anna“ wird zu „Anna Berg“. Leer -> `None`.
pub fn display_name(s: &str) -> Option<String> {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let joined = match joined.split_once(',') {
        Some((last, first))
            if !first.contains(',') && !last.trim().is_empty() && !first.trim().is_empty() =>
        {
            format!("{} {}", first.trim(), last.trim())
        }
        _ => joined,
    };
    let cut: String = joined
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let cut = cut.trim().to_string();
    (!cut.is_empty()).then_some(cut)
}

/// Firma = Domain der Adresse, ausser bei Freemail-Anbietern. Erwartet eine
/// schon normalisierte oder normalisierbare Adresse.
pub fn company_from_email(e: &str) -> Option<String> {
    let email = normalize_email(e)?;
    let domain = email.rsplit('@').next()?;
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    if FREEMAIL.contains(&domain) {
        return None;
    }
    Some(domain.to_string())
}

/// Notname aus der Adresse, wenn der Kalender keinen Namen liefert:
/// `anna.berg@firma.de` -> „Anna Berg“.
pub fn name_from_email(e: &str) -> Option<String> {
    let email = normalize_email(e)?;
    let local = email.split('@').next()?;
    let words: Vec<String> = local
        .split(['.', '_', '-', '+'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect();
    let name = words.join(" ");
    display_name(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_comma_first_equals_first_last() {
        assert_eq!(normalize_name("Berg, Anna"), "anna berg");
        assert_eq!(normalize_name("Anna Berg"), "anna berg");
        assert_eq!(normalize_name("  BERG ,   Anna  "), "anna berg");
        assert_eq!(normalize_name("\"Berg, Anna\""), "anna berg");
        assert_eq!(display_name("Berg, Anna").as_deref(), Some("Anna Berg"));
    }

    #[test]
    fn names_fold_diacritics_and_whitespace_but_do_not_guess() {
        assert_eq!(normalize_name("Ünal  Çelik"), "unal celik");
        assert_eq!(normalize_name("Müller"), normalize_name("MULLER"));
        assert_eq!(normalize_name("Straße"), "strasse");
        // Keine unscharfe Automatik: das sind zwei Schluessel.
        assert_ne!(normalize_name("Müller"), normalize_name("Mueller"));
        assert_ne!(normalize_name("Anna Berg"), normalize_name("A. Berg"));
        // Mehr als ein Komma ist kein „Nachname, Vorname“.
        assert_eq!(normalize_name("Meier, Huber, Co"), "meier, huber, co");
        assert_eq!(normalize_name("   "), "");
        assert_eq!(display_name("   "), None);
    }

    #[test]
    fn emails_are_lowercased_trimmed_and_validated() {
        assert_eq!(
            normalize_email("  MailTo:Anna.Berg@Firma.DE ").as_deref(),
            Some("anna.berg@firma.de")
        );
        for bad in [
            "",
            "anna",
            "anna@",
            "@firma.de",
            "a@b",
            "a@@b.de",
            "a b@c.de",
            "<a@b.de>",
            "a@b.de.",
        ] {
            assert_eq!(normalize_email(bad), None, "{bad}");
        }
    }

    #[test]
    fn freemail_domains_have_no_company() {
        assert_eq!(company_from_email("a@gmail.com"), None);
        assert_eq!(company_from_email("a@GMX.de"), None);
        assert_eq!(company_from_email("a@t-online.de"), None);
        assert_eq!(company_from_email("a@web.de"), None);
        assert_eq!(
            company_from_email("Anna@Firma-GmbH.de").as_deref(),
            Some("firma-gmbh.de")
        );
        assert_eq!(
            company_from_email("a@www.firma.de").as_deref(),
            Some("firma.de")
        );
        assert_eq!(company_from_email("kaputt"), None);
    }

    #[test]
    fn a_name_can_be_derived_from_an_address() {
        assert_eq!(
            name_from_email("anna.berg@firma.de").as_deref(),
            Some("Anna Berg")
        );
        assert_eq!(
            name_from_email("clara@example.org").as_deref(),
            Some("Clara")
        );
        assert_eq!(name_from_email("kaputt"), None);
    }
}
