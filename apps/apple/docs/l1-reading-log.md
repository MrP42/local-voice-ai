# Lesesitzungen und Fortschritt (#795, L1)

`fortschritt.json` führt je Buch `progress.reading:<buch-id>` als normalen
Fortschrittsdatensatz mit `offset: 0` und `reading_log`. Er reist unverändert
über die vorhandene Sync-Sammlung `progress`; ein eigener Hub-Endpunkt ist
nicht nötig. Kurzbuch-Einträge schreiben das Protokoll ihres übergeordneten
Buchs. Ihre Positionsschlüssel bleiben eigenständig.

Jede Sitzung enthält `start_ms`, `end_ms`, `edition`, `medium`
(`text|audio|live|kurzbuch`), `from`, `to`, `unit` und `device`. Die Gerätekennung
ist eine lokal dauerhaft gespeicherte Zufallskennung, unabhängig vom Konto.
Ab 30 Sekunden werden Zwischenstände derselben Sitzung aktualisiert. Pause,
Stop, Hintergrund und Verlassen sichern den Stand; laufendes Hintergrundaudio
setzt die Sitzung fort. Gerätekennung und Beginn bilden den Unionsschlüssel;
Ende und weiteste Position gewinnen jeweils unabhängig als Maximum.

Keine Sitzung wird gekappt. Das bestehende Sync-Limit von 256 KiB je Objekt
gilt weiter: übergroße Objekte werden abgewiesen, nicht abgeschnitten. Eine
spätere Aufteilung langer Protokolle müsste den Sync-Vertrag erweitern.

Fortschrittsbereiche und `frontier` steigen innerhalb derselben Koordinaten.
Unvereinbare Text-Hashes und Rohtext werden nicht vermischt: nicht aktive
Stände bleiben unter `coordinate_history` (`raw` oder `hash:<source_hash>`)
ohne rekursive Historien erhalten. Beim Zurückwechseln werden ihre Bereiche
wieder vereinigt. Die aktuelle Position bleibt unabhängig davon beweglich.
Unbekannte Fortschritts- und Sitzungsfelder bleiben beim Lesen/Schreiben erhalten.

EPUB-Beschreibungen kommen aus OPF `dc:description`, einschließlich CDATA,
HTML, Entities und normalisiertem Leerraum. Am Desktop hat diese Beschreibung
Vorrang vor dem bestehenden, vom Metadatenkandidaten ausgelösten Open-Library-
Abruf. Das iPhone besitzt bisher nur die Cover-Suche, keinen Werk-Abruf für
Beschreibungen; dafür wird kein neuer Netzwerkweg hinzugefügt.

## Prüfung und Übergabe

- Offline: `python -B apps/apple/scripts/test_l1_reading_contracts.py`;
  jede Anschlussprüfung besitzt eine Gegenprobe im Speicher.
- Desktop: in `apps/local-voice`:
  `node --test --test-isolation=none scripts/fortschritt-sync.test.mjs`.
  Die neuen Verhaltensfälle wurden vor der Behebung rot ausgeführt.
- Rust-Gate: Tests in `library.rs`, `sync/books.rs` und `ebook.rs`.
- Mac-Gate: VoiceCore-XCTest, insbesondere `LibraryTests`, `BookTextTests`,
  `LibrarySyncModelTests` und `LiveTranslationTests`.
- Vor TypeScript-Prüfung die Bindings erzeugen: neuer Specta-Befehl
  `library_epub_description`. `src/bindings.ts` wurde nicht manuell geändert.
- Keine neuen Swift-Dateien; `generate_project.py` benötigt keine Ergänzung.
- Cargo-/pnpm-Builds, XCTest und ein echter Geräteabgleich wurden hier nicht
  ausgeführt. Die Offline-Anschlussprüfungen ersetzen diese Gates nicht.
