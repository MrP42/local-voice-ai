//! lva#365 P4c-2: Die Gesprächswerkzeuge (`conversations`, `conversation_entries`, `conversation`) lesen
//! keine Projektgespräche. Ein Projektgespräch trägt Antworten aus Projektinhalten; es verlässt das Projekt
//! nur über das Versandtor, nicht über ein Werkzeug im gewöhnlichen Gespräch (und von dort an ein
//! Cloud-Modell). Nur synthetische Texte.

use super::*;
use crate::conversations::store::{NewEntry, OWNER_ID, VOICEBOT_ID};

const PROJEKT: &str = "prj_01HZZZZZZZZZZZZZZZZZZZZZZZ";
const GEHEIM: &str = "AKTENZEICHEN-77x-nur-im-Projekt";

fn beitrag(role: &str, text: &str) -> NewEntry {
    NewEntry {
        id: None,
        role: role.to_string(),
        mode: "typed".to_string(),
        text: text.to_string(),
        status: "complete".to_string(),
        created_at: None,
        author_id: Some(
            if role == "user" {
                OWNER_ID
            } else {
                VOICEBOT_ID
            }
            .to_string(),
        ),
        triggered_by: None,
        model: None,
        attachments: Vec::new(),
        variants: Vec::new(),
        active_variant: None,
        sources: Vec::new(),
        untrusted: None,
        reply_to: None,
        ext: None,
    }
}

fn id(n: u8) -> String {
    format!("conv_{:0>26}", n)
}

/// Gespräche, deren `scope` sich nicht sicher lesen lässt (roh geschrieben, wie aus einer fremden Fassung).
const UNLESBAR: [(u8, &str); 3] = [
    (1, "{ kaputt"),
    (
        2,
        r#"{"data_class":"INTERNAL","special_category":false,"project_id":5}"#,
    ),
    (3, "[]"),
];
/// Ein Gespräch aus einer älteren Fassung: `scope` ohne Projekt, also gewöhnlich.
const ALTFASSUNG: u8 = 4;

struct Szene {
    _dir: tempfile::TempDir,
    store: ConversationStore,
    gewoehnlich: String,
    projekt: String,
}

fn szene() -> Szene {
    let dir = tempfile::tempdir().unwrap();
    let store = ConversationStore::open_upgrading(&dir.path().join("c.db"), |conn| {
        let mut zeilen: Vec<(u8, String)> = UNLESBAR
            .iter()
            .map(|(n, s)| (*n, (*s).to_string()))
            .collect();
        zeilen.push((ALTFASSUNG, "{}".to_string()));
        for (n, scope) in zeilen {
            conn.execute(
                "INSERT INTO conversations (id, schema_version, ts, trace_id, title, created_at,
                    updated_at, kind, scope, participants, context, pinned, archived)
                 VALUES (?1, '0.1.3', 't', 'tr', ?3, '2026-01-01T00:00:00.000Z',
                    '2026-01-02T00:00:00.000Z', 'voice', ?2, '[]', '[]', 0, 0)",
                rusqlite::params![id(n), scope, format!("Altbestand {n}")],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO conversation_entries (id, conversation_id, schema_version, ts, trace_id,
                    role, mode, text, status, attachments, variants, sources, created_at)
                 VALUES (?1, ?2, '0.1.3', 't', 'tr', 'assistant', 'typed', ?3, 'complete', '[]', '[]',
                    '[]', '2026-01-02T00:00:00.000Z')",
                rusqlite::params![
                    format!("entry_alt{n:0>22}"),
                    id(n),
                    if n == ALTFASSUNG {
                        "Alter Beitrag".to_string()
                    } else {
                        format!("{GEHEIM} (roh {n})")
                    }
                ],
            )
            .unwrap();
        }
    })
    .unwrap();
    let gewoehnlich = store.create(Some("Gewöhnlich".into())).unwrap();
    store
        .append_entry(&gewoehnlich.id, beitrag("user", "Wie wird das Wetter?"))
        .unwrap();
    store
        .append_entry(&gewoehnlich.id, beitrag("assistant", "Sonnig und mild."))
        .unwrap();
    let projekt = store
        .create_in_project(Some("Im Projekt".into()), PROJEKT)
        .unwrap();
    store
        .append_entry(
            &projekt.id,
            beitrag("user", "Was steht in der Ausschreibung?"),
        )
        .unwrap();
    store
        .append_entry(
            &projekt.id,
            beitrag("assistant", &format!("Das Aktenzeichen ist {GEHEIM}.")),
        )
        .unwrap();
    Szene {
        _dir: dir,
        store,
        gewoehnlich: gewoehnlich.id,
        projekt: projekt.id,
    }
}

fn ausgeschlossen(s: &Szene) -> Vec<String> {
    let mut ids = vec![s.projekt.clone()];
    ids.extend(UNLESBAR.iter().map(|(n, _)| id(*n)));
    ids
}

#[test]
fn die_liste_der_gespraeche_nennt_keine_projektgespraeche() {
    let s = szene();
    let liste = gespraeche_ausserhalb_projekt(&s.store).unwrap();
    let ids: Vec<&str> = liste.iter().map(|c| c.id.as_str()).collect();
    assert!(ids.contains(&s.gewoehnlich.as_str()));
    assert!(
        ids.contains(&id(ALTFASSUNG).as_str()),
        "ein Gespräch aus einer älteren Fassung ist gewöhnlich"
    );
    for aus in ausgeschlossen(&s) {
        assert!(!ids.contains(&aus.as_str()), "{aus} darf nicht erscheinen");
    }
    // Weder der Titel noch der Anfang des letzten Beitrags eines Projektgesprächs steht in der Auskunft.
    let text = format!("{liste:?}");
    assert!(!text.contains(GEHEIM));
    assert!(!text.contains("Im Projekt"));
    assert!(!text.contains("Ausschreibung"));
}

#[test]
fn die_beitraege_nennen_keine_projektgespraeche() {
    let s = szene();
    let beitraege = beitraege_ausserhalb_projekt(&s.store, 100).unwrap();
    let texte: Vec<&str> = beitraege.iter().map(|b| b.text.as_str()).collect();
    assert!(texte.contains(&"Sonnig und mild."));
    assert!(texte.contains(&"Alter Beitrag"));
    for aus in ausgeschlossen(&s) {
        assert!(
            beitraege.iter().all(|b| b.conversation_id != aus),
            "{aus} darf nicht erscheinen"
        );
    }
    let alle = texte.join("\n");
    assert!(!alle.contains("Ausschreibung"));
    assert!(!alle.contains("Aktenzeichen ist"));
    for n in 1..=3 {
        assert!(!alle.contains(&format!("(roh {n})")));
    }
}

#[test]
fn der_einzelabruf_eines_projektgespraechs_ist_ein_fehlendes_gespraech() {
    let s = szene();
    let fehlt = gespraech_ausserhalb_projekt(&s.store, &id(99)).unwrap_err();
    assert_eq!(fehlt, GESPRAECH_FEHLT);
    for aus in ausgeschlossen(&s) {
        // Dieselbe Antwort wie bei einem fehlenden Gespräch: Sie verrät nicht, dass es eines gibt.
        assert_eq!(
            gespraech_ausserhalb_projekt(&s.store, &aus).unwrap_err(),
            fehlt,
            "{aus}"
        );
    }
    // Auch eine Kennung, die keine Gesprächskennung ist, bleibt ein fehlendes Gespräch.
    assert_eq!(
        gespraech_ausserhalb_projekt(&s.store, "x' OR '1'='1").unwrap_err(),
        fehlt
    );
}

#[test]
fn das_gewoehnliche_gespraech_bleibt_lesbar() {
    let s = szene();
    let v = gespraech_ausserhalb_projekt(&s.store, &s.gewoehnlich).unwrap();
    assert_eq!(v.title, "Gewöhnlich");
    let texte: Vec<&str> = v.entries.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texte, ["Wie wird das Wetter?", "Sonnig und mild."]);
    let liste = gespraeche_ausserhalb_projekt(&s.store).unwrap();
    let eigenes = liste.iter().find(|c| c.id == s.gewoehnlich).unwrap();
    assert_eq!(eigenes.entries, 2);
    assert_eq!(eigenes.last_text, "Sonnig und mild.");
}

#[test]
fn archivierte_gespraeche_bleiben_wie_bisher_ausgeblendet() {
    let s = szene();
    s.store
        .update(&s.gewoehnlich, None, None, Some(true))
        .unwrap();
    assert_eq!(
        gespraech_ausserhalb_projekt(&s.store, &s.gewoehnlich).unwrap_err(),
        GESPRAECH_FEHLT
    );
    let liste = gespraeche_ausserhalb_projekt(&s.store).unwrap();
    assert!(liste.iter().all(|c| c.id != s.gewoehnlich));
}

#[test]
fn das_projektgespraech_bleibt_in_der_oberflaeche_sichtbar() {
    // Die Auskunft für Werkzeuge ist enger als die Liste der Oberfläche: Die Person sieht ihr Projektgespräch.
    let s = szene();
    let alle = s.store.list(false).unwrap();
    assert!(alle.iter().any(|c| c.id == s.projekt));
    assert!(s.store.get(&s.projekt).is_ok());
}
