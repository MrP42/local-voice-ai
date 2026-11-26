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

Desktop und iPhone schreiben dasselbe Protokoll. Play/Pause, Umblättern,
Verlassen und Hintergrund sichern Checkpoints. Nur laufendes Audio zählt im
Hintergrund weiter. Die Mindestdauer wird monoton gemessen (`performance.now`
bzw. `ProcessInfo.systemUptime`); die Wanduhr liefert nur Start und Ende.

Nach jeder Union bleiben die neuesten 256 Sitzungen, sortiert nach `start_ms`
und bei Gleichstand nach `device`. Zusätzlich fallen die ältesten Einträge weg,
bis das JSON-Array höchstens 120 KiB UTF-8 belegt. Damit bleibt für den kleinen
`reading:`-Datensatz samt Sync-Umschlag und erneutem JSON-Escaping ausreichend
Reserve unter 256 KiB.
Innerhalb dieses Fensters bleiben Einträge und unabhängige Maxima erhalten;
außerhalb werden Sitzungen bewusst verworfen, nicht als Tagesstatistik gezählt.
Die Begrenzung gilt in Swift, TypeScript und im Rust-Sync. Positionsbereiche und
Lesegrenze werden dadurch nicht gekappt. Der Byteetat umfasst auch unbekannte
Sitzungsfelder; ein einzelner zu großer Eintrag fällt ebenfalls weg.

Fortschrittsbereiche und `frontier` steigen innerhalb derselben Koordinaten.
Unvereinbare Text-Hashes und Rohtext werden nicht vermischt: nicht aktive
Stände bleiben unter `coordinate_history` (`raw` oder `hash:<source_hash>`)
ohne rekursive Historien erhalten. Beim Zurückwechseln werden ihre Bereiche
wieder vereinigt. Die aktuelle Position bleibt unabhängig davon beweglich.
Unbekannte Fortschritts- und Sitzungsfelder bleiben beim Lesen/Schreiben erhalten.

Altclients können `coordinate_history` beim Schreiben am Hub entfernen; ohne
Protokolländerung lässt sich das nicht verhindern. Neue Clients vereinigen eine
noch lokal vorhandene Historie erneut. Fehlende Historie bedeutet kein Zurücksetzen:
bekannte `frontier`-, `read`- und `heard`-Werte derselben Koordinaten bleiben monoton.
Ist die Historie auf allen Geräten verloren, können fremde Koordinaten nicht
rekonstruiert werden. Es gibt keine Versionssperre. Bei vorhandenem Rohtext-Mapping
werden dessen Fortschritt und ein historischer Zielhash vereinigt; die Historie
darf neueren Desktop-Fortschritt nicht verdecken.

EPUB-Beschreibungen kommen aus OPF `dc:description`, einschließlich CDATA,
HTML, Entities und normalisiertem Leerraum. Am Desktop hat diese Beschreibung
Vorrang vor dem bestehenden, vom Metadatenkandidaten ausgelösten Open-Library-
Abruf. Das iPhone besitzt bisher nur die Cover-Suche, keinen Werk-Abruf für
Beschreibungen (`CoverLookup` fordert nur `title,author_name,cover_i` an;
`BookSearch` sucht lokal im Buchtext); dafür wird kein neuer Netzwerkweg hinzugefügt.
Der Beschreibungsrückfall bleibt auf dem iPhone offen.

Die HTML-Bereinigung entfernt Tags in linearen Durchläufen und verwirft
`script`-/`style`-Inhalte. Nach zwei Entity-Dekodierungen entfernt ein letzter
Durchlauf auch daraus entstandene Tags, ohne weitere Entity-Dekodierung.

## Prüfung und Übergabe

- Offline: `python -B apps/apple/scripts/test_l1_reading_contracts.py`;
  jede Anschlussprüfung besitzt eine Gegenprobe im Speicher.
- Desktop: in `apps/local-voice`:
  `node --test --test-isolation=none scripts/fortschritt-sync.test.mjs`.
  Die neuen Verhaltensfälle wurden vor der Behebung rot ausgeführt.
- Rust-Gate: Tests in `library.rs`, `sync/books.rs` und `ebook.rs`.
- Mac-Gate: VoiceCore-XCTest, insbesondere `LibraryTests`, `BookTextTests`,
  `LibrarySyncModelTests` und `LiveTranslationTests`.
- `src/bindings.ts` ist seit a7acf81a bereits erzeugt und bleibt hier unberührt.
- Keine neuen Swift-Dateien; `generate_project.py` benötigt keine Ergänzung.
- Cargo-/pnpm-Builds, XCTest und ein echter Geräteabgleich wurden hier nicht
  ausgeführt. Die Offline-Anschlussprüfungen ersetzen diese Gates nicht.
