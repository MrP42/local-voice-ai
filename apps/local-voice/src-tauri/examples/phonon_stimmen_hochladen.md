# Einmaliger Phonon-Upload für den Gerätebau (#898)

Das Example startet keine App und erzeugt keine App-Funktion. Es unterstützt die
installierte Windows-App. Es benutzt dieselben Windows Known Folders wie Tauri
und die Kennung aus `tauri.conf.json`:

- Stimmen und Modelle: Local-AppData / App-Kennung / `tts/phonon`, auf diesem PC
  `C:\Users\wolff\AppData\Local\de.wolffappliedai.localvoiceai\tts\phonon`.
- Konto: Roaming-AppData / App-Kennung / `sync.json`.
- Beide Modelle (`phonon-40m-de`, `phonon-31m-en`) müssen nach der bestehenden
  Installationsprüfung der App vollständig installiert sein.

Aufrufe aus der Repo-Wurzel (Build und Ausführung durch Thread-Claude):

```powershell
cargo run --locked --manifest-path apps/local-voice/src-tauri/Cargo.toml --example phonon_stimmen_hochladen
cargo run --locked --manifest-path apps/local-voice/src-tauri/Cargo.toml --example phonon_stimmen_hochladen -- --hochladen
```

Ohne Schalter: ausschließlich Lesen und Berechnen im Speicher, kein Netzwerk,
keine temporären Dateien, keine Reparatur des Stimmenindex. Die Ausgabe enthält
je Stimme nur Name, Sprache, acht Zeichen des Master-SHA-256, die erzeugten
emb-Fassungen für beide Modelle mit Bytezahl und die Größe des geplanten
kanonischen Klartextobjekts in Bytes. Das ist keine HTTP-/Chiffratgröße; ein
vorhandenes Hub-Objekt kann wegen erhaltener älterer Fassungen größer sein.
Fehler bei der Vorbereitung verhindern den gesamten Upload. Fehlende UIDs,
offener Empfang und abweichende Master-Hashes werden gemeldet statt repariert.
Portable-Daten werden nicht automatisch geöffnet; eine Portable-Marke neben dem
Example führt zum Abbruch.

Mit `--hochladen`: erst alle Stimmen vorbereiten und auflisten, dann die aktuellen
Hub-Stände lesen und prüfen. Geschrieben werden nur reservierte Stimm-Systemordner,
deren Stimmobjekte und die dazugehörigen verschlüsselten Original-Blobs. Die emb-
Fassungen liegen wie in der App inline im Stimmobjekt. Es gibt keine Blob-Löschung,
keine Grabsteine, keine Änderungen an anderen Sammlungen und keine DB-Migration.
Die Erzeugung der Stimmdatei und emb-Register, Objektprüfung, Kanonisierung,
Versiegelung, Blob-Reservierung und Blob-Übertragung verwenden die App-Helfer.

## Konto, Schlüssel, Ledger und laufende App

Quellenbefund im Code:

- `sync/account.rs`: `SyncConfig` enthält Hub-URL, Token, Benutzerkennung,
  Gerätekennung und den bereits abgeleiteten Objektschlüssel (`enc_key_b64`).
  Alles kommt aus Roaming-`sync.json`. Kein Passwort wird benötigt. Der Sync-
  Schlüssel kommt weder aus der Datenbank noch aus dem Keyring und ist nicht
  ausschließlich im Speicher der laufenden App verfügbar. Das Example liest
  diesen Datensatz einmal und benutzt `SyncConfig::key()` und `HubClient`.
- `managers/calendar/secret.rs`: `secrets/` enthält andere Zugänge, unter Windows
  mit DPAPI, unter macOS über einen Tresorschlüssel im Keyring geschützt. Das ist
  kein Eingang des vorhandenen Sync-Clients. Das Example liest keine Secrets.
- `sync/ledger.rs`: Roaming-`sync_ledger.json` gehört dem allgemeinen Sync und
  wird atomar ersetzt. `sync/ablage.rs`: der Ablage-Sync hat außerdem einen
  eigenen Zustand unter Roaming-`ablage/sync.json`, ebenfalls atomar ersetzt.
- `SyncEngine::account_guard`/`run_lock`, `VoiceStore::guard` und die Ablage-
  Sperren sind In-Process-Sperren. Sie koordinieren kein separates Example.
  Atomare Dateiersetzung allein schützt nicht vor verlorenen Änderungen durch
  zwei Prozesse. Auch `VoiceStore::read_index` und `Ablage::sync_bestand` können
  reparieren/schreiben und werden deshalb hier nicht auf produktiven Daten aufgerufen.

Das Example **schreibt keines der beiden Ledger und keine anderen lokalen Dateien**.
Ein paralleler Schreibzugriff auf den lokalen App-Bestand findet dadurch nicht
statt; ein Beenden der App ist nicht nötig. Revisionen stammen aus einem frischen
Hub-Pull und leben nur im Speicher. Der Hub benötigt kein lokales Ledger für einen
Push. Die App empfängt bestätigte Änderungen später über ihren normalen Pull.

Sicherheitsgrenzen: Der gesamte Mehr-Objekt-Upload ist keine Transaktion. Vor der
Übertragung und erneut vor Objekt-Pushes werden Konto, Index und Masterbytes auf
Änderung geprüft; ein erkannter Wechsel bricht ab. Der Hub prüft die Basisrevision
jedes Objekts (CAS). Eine parallele Hub-Änderung, Grabstein, fremder Schlüssel,
anderer Master oder abweichende Teilnahme führt zum Abbruch statt Überschreiben.
Es gibt keinen automatischen Retry, keine Generationserhöhung und kein Erzwingen.
Die Prüfungen sperren die laufende App nicht: eine nach einer Prüfung beginnende
Änderung bleibt durch CAS bzw. den normalen folgenden App-Abgleich aufzulösen.
Es wird ein geprüfter Snapshot übertragen, keine exklusive Gesamtsicht garantiert.
Bereits bestätigte Stimmen werden gezählt; bei einem Netzfehler können zusätzlich
Blobs oder Objekte angekommen sein, deren Bestätigung fehlt. Auch ein Ordner kann
bereits übertragen sein, bevor die erste Stimme bestätigt ist. Das Teilergebnis
weist darauf hin. Keine Serverfehlertexte, Tokens, Schlüssel, Stimm-/Audiobytes,
Base64-Daten oder Quellnotizen werden ausgegeben; ein Logger wird nicht installiert.

## Prüfauftrag für Thread-Claude

In diesem Umsetzungsthread wurden auftragsgemäß keine Cargo-/pnpm-Bauten,
Rust-Tests oder echten Uploads ausgeführt. Synthetische Rust-Tests sind vorhanden:

```powershell
cargo check --locked --manifest-path apps/local-voice/src-tauri/Cargo.toml --example phonon_stimmen_hochladen
cargo test --locked --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib einmalig
cargo test --locked --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib phonon
```

Die Tests prüfen Listenbildung mit zwei Projektionen, beschränkte Ausgabe,
unveränderte lokale Testdateien, fehlende UID/kaputte Stimme, expliziten Schalter,
Erhalt des emb-Rangs bei neuer Modellrevision, wiederholten Upload ohne zusätzliche
Fassungen, Begrenzung auf Systemobjekte, lokalen Kontowechsel, Hub-CAS-Konflikt
und Abbruch mit Teilergebnis. Der Transporttest nutzt den bestehenden FakeHub.

Noch auszuführen ist auch die Rot-Gegenprobe in einer isolierten Testkopie:

1. In `hochladen(args)` den Standard von `Ok(false)` auf `Ok(true)` ändern:
   `upload_braucht_den_ausdruecklichen_schalter` muss scheitern.
2. In `listenzeile` die Prüfung auf fehlende/fremde emb entfernen:
   `trockenlauf_listet_nur_metadaten_und_beide_emb_fassungen` muss scheitern.
3. Die CAS-/Bestätigungsprüfung in `stimmen_einmalig::push` umgehen:
   `einmaliger_upload_bricht_bei_cas_konflikt_und_nach_teilergebnis_ab` muss scheitern.
4. Mutationen verwerfen und dieselben Tests wieder grün ausführen.

Die realen Konto-/Stimmdateien wurden für die Implementierung nicht geöffnet.
Windows-Known-Folder-Auflösung, tatsächliche Modellinstallation, TLS/Hub-Verhalten
und paralleler App-Abgleich brauchen noch den lokalen Lauf durch Thread-Claude.
