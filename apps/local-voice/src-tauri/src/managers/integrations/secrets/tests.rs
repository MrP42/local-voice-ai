use super::*;
use crate::managers::calendar::secret::Namespace;
use crate::managers::integrations::model::{Direction, Integration};

fn integration(id: &str, kind: Kind) -> Integration {
    Integration {
        id: id.into(),
        kind,
        label: "Test".into(),
        enabled: true,
        direction: Direction::Both,
        config_json: "{}".into(),
        account_hint: None,
        data_class: None,
        created_at: 0,
        updated_at: 0,
        last_ok_at: None,
        last_error: None,
    }
}

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

const TOKEN: &str = "0.AAAA-refresh-token-GEHEIM-0123456789";

#[test]
fn calendar_kinds_keep_their_old_secret_name_and_namespace() {
    for (id, kind) in [
        ("01K8Z3Q6M2V7N4T9X5B1C8D0EF", Kind::Ics),
        ("graph-3fa9c1d27be04a58", Kind::Graph),
    ] {
        let r = secret_ref(&integration(id, kind), "token");
        assert_eq!(r.ns, Namespace::Calendar, "{kind:?}");
        assert_eq!(
            r.name().unwrap(),
            id,
            "Dateiname wie bisher, kein Neu-Login"
        );
    }
}

#[test]
fn other_kinds_get_the_integration_namespace_with_a_slot() {
    let r = secret_ref(&integration("01JABC", Kind::Smtp), "password");
    assert_eq!(r.ns, Namespace::Integration);
    assert_eq!(r.name().unwrap(), "int-01JABC-password");
    let r = secret_ref(&integration("mein-vault", Kind::Obsidian), "token");
    assert_eq!(r.name().unwrap(), "int-mein-vault-token");
}

#[test]
fn names_that_could_leave_the_namespace_are_refused() {
    let long_id = "x".repeat(64);
    for (id, slot) in [
        ("", "token"),
        ("../x", "token"),
        ("a/b", "token"),
        ("a b", "token"),
        ("ok", ""),
        ("ok", "Token"),
        ("ok", "to-ken"),
        ("ok", "to.ken"),
        ("ok", "a-very-long-slot-name-that-goes-on"),
        (long_id.as_str(), "token"),
    ] {
        assert!(
            SecretRef::integration(id, slot).name().is_err(),
            "{id:?} {slot:?}"
        );
    }
    // Der Kalenderraum kennt nur das Fach `main` und nie die Vorsilbe `int-`.
    assert!(SecretRef {
        ns: Namespace::Calendar,
        id: "x".into(),
        slot: "token".into()
    }
    .name()
    .is_err());
    assert!(SecretRef::calendar("int-eins-token").name().is_err());
    assert!(SecretRef::calendar("").name().is_err());
    assert!(SecretRef::calendar("../x").name().is_err());
}

#[test]
fn the_legacy_calendar_writer_cannot_reach_the_integration_namespace() {
    let dir = tmp();
    let err =
        crate::managers::calendar::secret::put_in(dir.path(), "int-x-token", b"x").unwrap_err();
    assert!(err.contains("Ungültiger Name"), "{err}");
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[test]
fn a_missing_secret_is_reported_as_missing_not_as_an_error() {
    let dir = tmp();
    let i = integration("01JABC", Kind::Smtp);
    assert_eq!(status_in(dir.path(), &i, "password"), SecretStatus::Missing);
    // Auch im noch nicht angelegten Ordner.
    assert_eq!(
        status_in(&dir.path().join("gibt-es-nicht"), &i, "password"),
        SecretStatus::Missing
    );
    let all = statuses_in(dir.path(), &i);
    assert_eq!(all.len(), 1);
    assert_eq!((all[0].0, status_label(&all[0].1)), ("password", "missing"));
    // Arten ohne Geheimnis haben keine Faecher.
    assert!(statuses_in(dir.path(), &integration("f", Kind::Folder)).is_empty());
}

#[test]
fn a_foreign_file_without_the_magic_is_broken_with_a_plain_message() {
    let dir = tmp();
    let i = integration("01JABC", Kind::Smtp);
    std::fs::write(
        dir.path().join("int-01JABC-password.bin"),
        b"kein Geheimnis",
    )
    .unwrap();
    match status_in(dir.path(), &i, "password") {
        SecretStatus::Broken(msg) => {
            assert!(msg.contains("beschädigt"), "{msg}");
            assert!(
                !msg.contains("kein Geheimnis"),
                "die Meldung nennt keinen Inhalt"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn delete_all_removes_only_the_slots_of_that_integration() {
    let dir = tmp();
    for name in [
        "int-a-token.bin",
        "int-a-password.bin",
        "int-a-b-token.bin", // gehoert der ID `a-b`
        "int-ab-token.bin",
        "a.bin",
        "graph-1.bin",
        "int-a-token.txt",
    ] {
        std::fs::write(dir.path().join(name), b"x").unwrap();
    }
    assert_eq!(
        crate::managers::calendar::secret::delete_all_for_integration_in(dir.path(), "a"),
        2
    );
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec![
            "a.bin",
            "graph-1.bin",
            "int-a-b-token.bin",
            "int-a-token.txt",
            "int-ab-token.bin"
        ]
    );
    // Ungueltige IDs loeschen nichts; ein fehlender Ordner ist kein Fehler.
    assert_eq!(
        crate::managers::calendar::secret::delete_all_for_integration_in(dir.path(), "../a"),
        0
    );
    assert_eq!(
        crate::managers::calendar::secret::delete_all_for_integration_in(
            &dir.path().join("nope"),
            "a"
        ),
        0
    );
}

#[test]
fn deleting_an_integration_leaves_calendar_secrets_to_the_calendar() {
    let dir = tmp();
    std::fs::write(dir.path().join("graph-1.bin"), b"x").unwrap();
    assert_eq!(
        delete_all_in(dir.path(), &integration("graph-1", Kind::Graph)),
        0
    );
    assert!(dir.path().join("graph-1.bin").exists());
}

#[cfg(windows)]
mod dpapi {
    use super::*;

    #[test]
    fn a_calendar_secret_written_the_old_way_is_read_through_the_register() {
        let dir = tmp();
        // So hat der Kalender es bisher abgelegt.
        crate::managers::calendar::secret::put_in(
            dir.path(),
            "graph-3fa9c1d27be04a58",
            TOKEN.as_bytes(),
        )
        .unwrap();
        let i = integration("graph-3fa9c1d27be04a58", Kind::Graph);
        assert_eq!(status_in(dir.path(), &i, "main"), SecretStatus::Present);
        let got =
            crate::managers::calendar::secret::get_ref_in(dir.path(), &secret_ref(&i, "main"))
                .unwrap()
                .unwrap();
        assert_eq!(got.as_slice(), TOKEN.as_bytes());
    }

    #[test]
    fn an_integration_secret_round_trips_and_leaves_no_plaintext_on_disk() {
        let dir = tmp();
        let i = integration("01JABC", Kind::Smtp);
        put_in(dir.path(), &i, "password", TOKEN.as_bytes()).unwrap();
        assert_eq!(status_in(dir.path(), &i, "password"), SecretStatus::Present);
        let got =
            crate::managers::calendar::secret::get_ref_in(dir.path(), &secret_ref(&i, "password"))
                .unwrap()
                .unwrap();
        assert_eq!(got.as_slice(), TOKEN.as_bytes());
        let raw = std::fs::read(dir.path().join("int-01JABC-password.bin")).unwrap();
        for needle in ["GEHEIM", "refresh-token", "0.AAAA"] {
            assert!(
                !raw.windows(needle.len()).any(|w| w == needle.as_bytes()),
                "{needle}"
            );
        }
        // Kein Rest einer Temp-Datei.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["int-01JABC-password.bin".to_string()]);
    }

    #[test]
    fn a_blob_cannot_be_moved_between_namespaces_or_slots() {
        let dir = tmp();
        let smtp = integration("01JABC", Kind::Smtp);
        put_in(dir.path(), &smtp, "password", TOKEN.as_bytes()).unwrap();
        // In einen anderen Namen kopiert: nicht entschluesselbar.
        std::fs::copy(
            dir.path().join("int-01JABC-password.bin"),
            dir.path().join("int-01JABC-token.bin"),
        )
        .unwrap();
        assert!(matches!(
            status_in(dir.path(), &smtp, "token"),
            SecretStatus::Broken(_)
        ));
        // Als Kalendergeheimnis ausgegeben: ebenfalls nicht.
        std::fs::copy(
            dir.path().join("int-01JABC-password.bin"),
            dir.path().join("graph-9.bin"),
        )
        .unwrap();
        let graph = integration("graph-9", Kind::Graph);
        assert!(matches!(
            status_in(dir.path(), &graph, "main"),
            SecretStatus::Broken(_)
        ));
        // Und ein Kalendergeheimnis nicht als Integrationsgeheimnis.
        crate::managers::calendar::secret::put_in(dir.path(), "src-1", b"https://x.invalid/a")
            .unwrap();
        std::fs::copy(
            dir.path().join("src-1.bin"),
            dir.path().join("int-01JABC-main.bin"),
        )
        .unwrap();
        assert!(matches!(
            status_in(dir.path(), &smtp, "main"),
            SecretStatus::Broken(_)
        ));
    }

    #[test]
    fn a_damaged_blob_is_broken_with_a_message_that_holds_no_secret() {
        let dir = tmp();
        let i = integration("01JABC", Kind::Smtp);
        put_in(dir.path(), &i, "password", TOKEN.as_bytes()).unwrap();
        let path = dir.path().join("int-01JABC-password.bin");
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xFF;
        raw[8 + 40] ^= 0xFF;
        std::fs::write(&path, &raw).unwrap();
        match status_in(dir.path(), &i, "password") {
            SecretStatus::Broken(msg) => {
                assert!(msg.contains("neu eingeben"), "{msg}");
                assert!(!msg.contains("GEHEIM"), "{msg}");
            }
            other => panic!("{other:?}"),
        }
        // Abgeschnitten.
        std::fs::write(&path, &raw[..8]).unwrap();
        assert!(matches!(
            status_in(dir.path(), &i, "password"),
            SecretStatus::Broken(_)
        ));
    }

    #[test]
    fn overwriting_replaces_atomically_and_delete_all_clears_every_slot() {
        let dir = tmp();
        let i = integration("01JABC", Kind::M365);
        put_in(dir.path(), &i, "token", b"eins").unwrap();
        put_in(dir.path(), &i, "token", b"zwei").unwrap();
        put_in(dir.path(), &i, "other", b"drei").unwrap();
        let got =
            crate::managers::calendar::secret::get_ref_in(dir.path(), &secret_ref(&i, "token"))
                .unwrap()
                .unwrap();
        assert_eq!(got.as_slice(), b"zwei");
        assert_eq!(delete_all_in(dir.path(), &i), 2);
        assert_eq!(status_in(dir.path(), &i, "token"), SecretStatus::Missing);
    }
}
