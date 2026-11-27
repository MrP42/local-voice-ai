# lva#954 – Übergabe Pakete 1, 3, 4, 5

## API für den UI-Thread (Paket 2)

`LibraryPlayback.scriptPresentation(inParagraph: index)` liefert optional eine
`SpeakerScript.Presentation`:

- `hiddenRanges`: auszublendende Marker und Tags, relativ zu
  `playback.paragraphs[index].text`, in UTF-16 (NSRange). Bereiche können sich
  überlappen, etwa bei `[<Name>]`.
- `sections`: Abschnittsbereiche in denselben Koordinaten, mit `speakerName`
  (nil vor dem ersten Marker) und `known`. Sprecher werden über Absatzgrenzen
  geerbt. Ein unbekannter Spitzklammer-Marker bekommt seinen Namen und
  `known == false`; er setzt die Synthese auf die Buchstimme zurück.

Der Text wird nicht verändert. `textStart`, Hash, TextOffsetMap, Suche und
Fortschritt behalten den bestehenden Rohtextvertrag. Farben/Labels und das
Ausblendeattribut implementiert Paket 2. Für andere Views steht die reine API
`SpeakerScript(raw, speakers: roster).presentation(in: paragraphRange)` bereit.

## Entscheidungen

- Sprecher stammen aus installierten Phonon-Stimmen einschließlich eigener
  Stimmen. `default` entfällt; Kurzname, Kennungsalias und Dubletten folgen
  `providers/speakers.rs`. Im Systemweg kommen installierte Systemnamen hinzu.
- Es gibt in der geprüften Spiegelung keine explizite Sprecherzuordnung:
  `sync/books.rs` spiegelt `tts.voice.lang.*`/`tts.voice.book.*`, `VoicePref`
  enthält desktop/ios/phonon/updated. Provider-`voice_names` ist kein solcher
  synchronisierter Sprecher→Stimme-Eintrag. Keine neue Persistenz eingeführt.
- Stil wird im Parser erhalten, aber nicht an Phonon übergeben. Die vorhandene
  iOS-Anbindung belegt nur `setVoice`, `stream` und `synthesize`, keine
  Emotionsschnittstelle. Tags werden vor der Synthese entfernt; die vier
  Desktop-Pausentags erzeugen Stille, auch am Anfang/Ende und direkt nacheinander.
- Phonon behält eine Fortschrittsmarke je Rohsatz. Stimme ist Teil des bestehenden
  begrenzten Audio-Caches; Vorberechnung berücksichtigt Sprecher und Pausen.
  Stimmwechsel laufen in nichtisolierten async-Synthesehelfern. Der Audio-Player,
  seine Warteschlange und seine Shutdown-/Stall-Mechanik sind unberührt.
- Systemskripte verwenden Utterances pro Sprechstück. Wörter werden über
  `sourceOffsets` zum Rohtext zurückgeführt. Pause verwirft einen Skriptsatz
  samt Regiepause; Weiter beginnt am Satzanfang.
- Live-Übersetzung schützt Marker und Tags als Literale. Zielsprache verteilt
  bekannte Sprecher wie `suggestSwap` auf vorhandene Zielstimmen; fehlt eine,
  gilt die eingestellte Buchstimme. Originalpositionen ändern sich dadurch nicht.

## Prüfungen und nächste Tore

Windows, ohne Build: `python -B apps/apple/scripts/test_speaker_script_contracts.py`.
Das prüft nur Quellverträge, Projektzugehörigkeit und Golden-Offsetplausibilität;
entfernte Anbindungen müssen in den Gegenproben scheitern. Verhalten prüft XCTest.

Rust-Thread, aus `apps/local-voice/src-tauri`:

```powershell
$env:UPDATE_GOLDEN = '1'
cargo test --lib speaker_script_golden
Remove-Item Env:UPDATE_GOLDEN
cargo test --lib speaker_script_golden
```

Das eingecheckte JSON enthält initial gesetzte Sollwerte. Erst der ausstehende
Rust-Erzeugungslauf bestätigt/ersetzt diese aus den fünf produktiven Funktionen;
die Swift-Tests lesen genau dieselbe Datei, ohne eigenes Golden oder Kopie.

Mac: Projekt mit `apps/apple/scripts/generate_project.py` erzeugen,
VoiceCore `swift test --filter SpeakerScriptTests`, danach iOS-Gerätebau mit
Phonon. Die Vendor-Swiftquellen sind in diesem Windows-Checkout nicht vorhanden;
deren Actor-/Sendable-Vertrag und der Gerätepfad müssen dort geprüft werden.
Keine Swift-/Rust-/pnpm-Bauten in diesem Auftrag ausgeführt.

Offene Abnahme: Screenshot-Skript mit zwei hörbaren Stimmen und 700-ms-Break,
unbekannter Sprecher, eigene Stimme, Systemweg, Live-Zielsprache, Sprung/
Fortsetzen mit Rohtextoffsets, Abbruch während Pause/Stimmwechsel, Bilder aus
Paket 2. Kein Merge-JA vor diesen Toren.
