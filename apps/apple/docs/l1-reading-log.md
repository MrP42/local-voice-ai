# Lesesitzungen und Fortschritt (#795, L1)

`fortschritt.json` führt je Buch `progress.reading:<buch-id>` als normalen
Fortschrittsdatensatz mit `offset: 0` und `reading_log`. Historie und Protokoll
reisen über die eigene Sync-Sammlung `reading_history`; die Positionsdaten
bleiben in `progress`. Ein eigener Hub-Endpunkt ist nicht nötig.
Kurzbuch-Einträge schreiben das Protokoll ihres übergeordneten
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
bis ihr gemeinsamer JSON-Etat höchstens 120 KiB belegt. Dieser obere Etat zählt
JSON-Strukturzeichen, Strings als UTF-8 mit JSON- und Slash-Escaping, jede Zahl
mit 32 Bytes, jeden booleschen Wert mit 5 und `null` mit 4 Bytes. Feldreihenfolge
und die Encoder-Schreibweisen `31000`/`31000.0` ändern ihn nicht. Unbekannte
Felder zählen rekursiv mit. Das tatsächliche JSON bleibt unter diesem Etat.
Die gemeinsame Fixture `apps/local-voice/scripts/fixtures/reading-log-budget.json`
prüft dieselbe Auswahl in Node, Rust und Swift, einschließlich einer Grenze
mit einem Zeichen Abstand und einer Unicode-/Escaping-Probe.
Damit bleibt für den kleinen
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

Altclients können `coordinate_history` in `progress` beim Ersatzschreiben verlieren.
Deshalb schreibt jeder neue Client `reading_history/<buch-id>` mit
`{book: <buch-id>, records: {<fortschrittsschlüssel>: <datensatz>}}`. Die
Datensätze enthalten auch den aktiven Koordinatenstand, damit die bestehende
Union unverändert angewendet werden kann. Neue `progress`-Nutzlasten lassen
`coordinate_history` und `reading_log` weg; `frontier`, `read` und `heard`
bleiben monoton je Koordinatensystem. Lokal bleiben die Felder für Wiedergabe
und Koordinatenwechsel verfügbar. Vorhandene eingebettete Felder werden beim
Lesen übernommen und beim nächsten Sammeln in die neue Sammlung migriert.
Eine auf allen Geräten bereits verlorene Historie lässt sich nicht rekonstruieren.

Die Kompatibilität gilt für passende Schlüsselkennung. Belege am Altstand
`d3edb3203` (Zeilennummern dieses Stands): Desktop `sync/engine.rs:851–853`
prüft `key_id` vor der Sammlungsverzweigung; bei Abweichung stoppt der Pull.
Mit passender Kennung gibt der unbekannte Zweig in `engine.rs:1244` erfolgreich
`Ok(changed(false))` zurück. iPhone `LibrarySync.swift:271–272` ignoriert
unbekannte Sammlungen vor Entschlüsselung und Decoder. Der Desktop kann ihre
Revision im Ledger verbuchen; seine feste Sammlungsmenge für Grabsteine und
sein Sammler erzeugen dafür weder Löschung noch Ersatzschreiben. Das iPhone
verbucht unbekannte Objekte gar nicht. Beide setzen den Pull fort.
Ein unbekannter Schlüssel innerhalb von `progress` hätte diese Isolation nicht.

Die neue Sammlung nutzt exakt den regulären Verschlüsselungsweg: Desktop
`PushObject.key_id = my_key_id` aus `crypto::key_id(cfg.key())`, iPhone
`sealObject(..., acc)` mit `keyId: acc.keyId`, wie `progress`. Die AAD enthält
weiterhin die jeweilige Sammlung und Objektkennung. Ein `key_mismatch` betrifft
alle Sammlungen gleichermaßen und ist kein neues Risiko.

Beim Upgrade wird vor dem ersten Push einmal ab Cursor 0 eingelesen. Beide
Ledger führen dafür eine eigene Historien-Marke (Desktop `reading_history_pulled`,
iPhone `readingHistoryPulled`), die erst nach vollständigem erfolgreichem Pull gilt.
Alte Ledger-Einträge für die zuvor unbekannte Sammlung werden vorher entfernt;
auch eigene Echos werden erst vereinigt und danach mit ihrer Revision verbucht.
Historien haben eigene Revisionen, Hashes und Konfliktbehandlung im Ledger.
Ein vereinigter Stand bleibt gegenüber dem empfangenen Hash sendebedürftig.
Der lokale Spiegel `reading_history.<buch-id>` in `fortschritt.json` erhält
die Zuordnung auch dann, wenn die Buchmetadaten später eintreffen. Lose Seiten
werden zunächst unter ihrer Seitenkennung geführt. Historien senden keine
automatischen Grabsteine; fehlender Fortschritt ist kein Löschauftrag.
Die Grenze von 256 KiB gilt für das gesamte Buchobjekt. Übergröße wird nicht
durch Kappen von Koordinatenständen gelöst; nur dieser Stand wird als `too_large`
zurückgehalten. Andere Historien, Fortschritte und Stimmen werden weiter gesendet.
Die Hub-Zulassung der neuen Sammlung und ein realer Geräteabgleich sind ein
separates Integrationsgate; der Hub-Server liegt nicht in diesem Repo.

Desktop-Audio wird im dauerhaften Ereignisempfänger `readerSession.ts`
aufgezeichnet. Buch, Fassung, Medium und Text-Locator werden beim Start
festgehalten. Satzmeldungen und der Checkpoint-Timer laufen auch bei anderem
Reiter und ausgehängter Ansicht weiter; Pause beendet den Checkpoint, Fortsetzen
beginnt einen neuen, Stopp verwirft die Quelle. Der React-Hook erfasst nur
sichtbares Lesen bei angehaltener Wiedergabe.

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
- Kompatibilität: `python -B apps/apple/scripts/test_reading_history_sync.py`;
  nachgebildeter Decoder/Schreiber von `d3edb3203`, synthetische Daten und
  Gegenprobe mit eingebetteter Historie. Rust/XCTest prüfen zusätzlich die
  produktive Union beider Sammlungen in beiden Empfangsreihenfolgen.
- Desktop: in `apps/local-voice`:
  `node --test --test-isolation=none scripts/fortschritt-sync.test.mjs`.
  Zusätzlich `node --test --test-isolation=none scripts/reading-playback.test.mjs`.
  Die neuen Verhaltensfälle wurden vor der Behebung rot ausgeführt.
- Rust-Gate: Tests in `library.rs`, `sync/books.rs` und `ebook.rs`.
- Mac-Gate: VoiceCore-XCTest, insbesondere `LibraryTests`, `BookTextTests`,
  `LibrarySyncModelTests` und `LiveTranslationTests`.
- `src/bindings.ts` ist seit a7acf81a bereits erzeugt und bleibt hier unberührt.
- Keine neuen Swift-Dateien; `generate_project.py` benötigt keine Ergänzung.
- Cargo-/pnpm-Builds, XCTest und ein echter Geräteabgleich wurden hier nicht
  ausgeführt. Die Offline-Anschlussprüfungen ersetzen diese Gates nicht.
