# PC-Relay model_chat (#801 T4c, #804)

Gemeinsame Wire-Typen: `VoiceCore/Models/RelayChat.swift` und am PC
`src-tauri/src/sync/model_chat.rs` (Kindmodul von `booktalk_relay`). Gemeinsame
synthetische HMAC-Vektoren: `tooling/fixtures/model-chat/golden.json`.

Transport: vorhandene Sammlung `booktalk_relay`, Ende-zu-Ende-Verschluesselung
mit deren AAD und Kontoschluessel. Eigene Objekte `model_models_<PC>_<deviceKey>`,
`model_req_<UUID>`, `model_resp_<UUID>`. Alle Wire-Felder ausser `mac` sind
HMAC-signiert, auch Metadaten, Modellliste und Verbrauch. Kanonisches JSON:
UTF-8-Schluessel sortiert, BookTalk-Zeichenmaskierung, keine Fliesskommazahlen;
Temperatur als Tausendstel. Listen und Anfragen gelten 90 Sekunden; erste
Antwort nach hoechstens 15, insgesamt 75 Sekunden. PC-Aufruf maximal 60 Sekunden.

Antworten enthalten alle bisherigen Delta-Stuecke mit fortlaufender `seq` ab 1.
Ein uebersprungener Poll verliert dadurch keinen Text. Luecken, doppelte oder
geaenderte Stuecke sind Fehler. Antworten bleiben bis zur Abholung erhalten;
Abschluss, Abbruch und Frist raeumen Anfrage und Antwort ab. Der PC merkt
ausstehende Loeschungen wie BookTalk in der Freigabeablage vor.

Freigabe ist je Kopplung standardmaessig AUS. `model_chat_relay_pair` erzeugt
einen BookTalk-kompatiblen Einrichtungscode auch fuer PCs ohne Abo-CLI.
`model_chat_relay_authorize(deviceKey, modelIds)` erlaubt einer bestehenden
Kopplung explizite PC-Modellkonfigurationen; eine leere Liste widerruft die
Funktion. Die vorhandene Geraete-Entkopplung entfernt auch diese Freigabe.
Im Store stehen Modellrechte im eigenen Feld `model_chat_models`, getrennt von
den bestehenden BookTalk-Rechten. Auch gleichnamige Modelle koennen dadurch
keine Freigabe der anderen Funktion erben; alte Kopplungen haben ein leeres Feld.
Ein erneuerter BookTalk-Code erfordert erneute model_chat-Freigabe am PC.

`provider` ist die PC-Verbindungskennung, `model` die stabile Modellkonfiguration
dieser Verbindung. So bleiben gleiche Remote-Modellnamen an verschiedenen
Verbindungen getrennt. Label, Anbieter und belegte Region kommen aus dem PC;
unbekannte Regionen bleiben leer. Alle PC-Routen gelten auf dem iPhone als extern,
auch wenn der PC ein lokales Modell verwendet.

`configuration_fingerprint` bindet den signierten Katalogeintrag und die signierte
Anfrage an Verbindung, tatsaechliche Zieladresse, Remote-Modell, Modelloptionen
und Anbieterdaten (SHA-256 des kanonischen JSON, ohne Zugangsdaten). Der PC
vergleicht beim Annehmen und nochmals nach wartender CLI-Vorbereitung unmittelbar
vor Versand. Abweichung: `configuration_changed`, kein Versand; das iPhone
verwirft den Katalog. Der Fingerabdruck ist zugleich Teil der Zustimmung und wird
vor dem Upload gegen deren Berechtigung geprueft. Ein neuer Zielstand braucht
einen neuen Katalog und neue Zustimmung. Das Pflichtfeld verlangt aktualisierte
PC- und iPhone-Versionen; alte Wire-Objekte werden nicht ausgefuehrt.

Ausfuehrung ueber den vorhandenen Anbieter-Router und llm_versand: Regelwerk,
Budget, Buchung und Abo-Vorschaupruefung bleiben verbindlich. Kein Modellwechsel.
Budget und Buchung erhalten die ausdruecklich gewaehlte Verbindung samt
Modellkonfiguration im Task-Kontext, einschliesslich deren Preise; das global
aktive PC-Modell bestimmt sie nicht. Ein CLI-Abbruch wartet den Blocking-Aufruf
und damit das Prozessende ab, bevor Buchung und Slot-Freigabe erfolgen. Ein noch
eintreffender erfolgreicher Verbrauch wird vor dem Verwerfen uebernommen. Liefert
die beendete CLI keinen auswertbaren Verbrauch, werden keine Token erfunden.
Der bestehende Versandweg ist noch die Uebergangsfassade (Inventar B14): die
PC-Besprechungs-Quellenpruefung von #601 ist damit nicht nachgewiesen; die
iPhone-Zustimmung wird durch ModelGate unmittelbar vor dem Upload geprueft.
API-Temperatur wird weitergegeben; Abo-CLIs bestimmen ihr Sampling selbst.
Schema wird fuer alle Wege als zusaetzliche Systemnachricht uebergeben.
Maximal zwei parallele Laeufe, einer je Geraet, 30/Stunde und 200/Tag je Geraet;
maxTokens am Backend: talk 800, meetingMinutes 4000, meetingSummary 1500,
textPrep 2000. liveState ist im Wire nicht erlaubt. Keine neue sichtbare UI.
Wire-Objekte sind auf 180000 Byte begrenzt; Nachrichten samt Schema auf 120000
UTF-8-Byte, Systemnachrichten samt Schema auf 8000 Byte.

## Abnahme durch Thread-Claude

Bereits ausfuehrbar ohne Installation/Build:

```
python apps/apple/scripts/test_model_chat_contracts.py
python apps/apple/scripts/test_endpoint_contracts.py
```

Noch auszufuehren (dieser Auftrag erlaubt hier keine Builds):

```
cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml model_chat
cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml booktalk_relay
cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml relay_abbruch
cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml relay_cli_abbruch
cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml explicit_selection_checks_and_books_b_despite_global_a
swift test --package-path apps/apple/VoiceCore --filter RelayChatTests
```

Zusaetzlich iPhone mit dem aktualisierten Xcode-Projekt bauen und den echten
verschluesselten Hin-/Rueckweg, Abo-CLI-Abbruch und Verbrauch pruefen. Die
Offline-Pruefungen enthalten Mutationen im Speicher als Gegenproben; die
Rust-/Swift-Verhaltenstests enthalten synthetische positive Kontrollen.

Fixrunde 1: Die Offline-Verdrahtungspruefung mutiert Replay-API, Auswahlkontext,
Fingerprint-Vergleiche, Zustimmungsbindung und CLI-Wartepfad ausschliesslich im
Speicher; jede Mutation muss rot werden. Sie ersetzt keine Uebersetzung und keine
Verhaltenstests. Fuer die nativen Gegenproben jeweils nur die zugehoerige Behebung
zuruecknehmen: `call_selection` ohne Task-Kontext muss den Budget-/Preistest
brechen; `matching_model` ohne Fingerprint-Vergleich den Anfrage-Test; eine
Zustimmung ohne Fingerprint oder fehlender Vergleich beim Upload die Swift-Tests;
der alte CLI-`select!` den kontrollierten Kindprozess-Test. Diese nativen Laeufe
und Gegenproben bleiben Thread-Claude ueberlassen.

## Geaenderte Dateien

- VoiceCore: `Models/RelayChat.swift`, `Tests/VoiceCoreTests/RelayChatTests.swift`.
- iPhone: `Models/RelayModelProvider.swift`, `Models/ModelCatalog.swift`,
  `BookTalk/BookTalkRelayHub.swift`, `Library/LibrarySync.swift`.
- Apple-Projekt und Abnahme: `LocalVoice.xcodeproj/project.pbxproj`,
  `scripts/test_model_chat_contracts.py`, `docs/model-chat-relay.md`.
- Desktop unter `src-tauri/src/`: `sync/model_chat.rs`, `sync/model_chat_tests.rs`,
  `sync/booktalk_relay.rs`, `lib.rs`, `llm_client.rs`, `llm_versand/uebergang.rs`,
  `sync/booktalk_relay/tests.rs`, `llm_versand/versand.rs`, `llm_versand/versand_tests.rs`.
- Desktop ausserhalb Rust: `src/bindings.ts`, `docs/llm-versand-inventar.md`.
- Gemeinsame Vektoren: `tooling/fixtures/model-chat/golden.json`.
