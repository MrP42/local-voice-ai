# lva#786 S1: Umsetzung und Übergabe

Auftrag vom 25.11.2026: ausschließlich Swift/VoiceCore und iPhone-Sync;
keine UI, kein Eingriff in Kamera, OCR oder Bibliotheksimport. Keine neuen
Abhängigkeiten, kein Desktop-Diff, keine Git-Schreibbefehle oder Netzaufrufe.

## Plan und Stand

- [x] Desktop-Vertrag und Apple-Speicher-/Kryptomuster nachlesen.
- [x] VoiceCore-Tests mit synthetischem Desktop-JSON vor dem Produktcode anlegen.
- [x] ReadingPage, Aufnahme-Metadaten und atomaren App-Gruppen-Speicher ergänzen.
- [x] Sammlung `page` in vorhandenen Pull/Push und Projektgenerator aufnehmen.
- [x] Offline-Projektprüfung samt zwei Mutationen ausführen; Diff/LF prüfen.
- [ ] Mac: VoiceCore-Suite, Swift-Gegenproben und iPhone-Build ausführen.

Die Umsetzung ist geschrieben, die Swift-Verifikation steht aus. Hier wurde
weder kompiliert noch eine erfolgreiche Swift-Rot/Grün-Gegenprobe beobachtet.

## Verträge und Entscheidungen

`ReadingPage` liest/schreibt `{title, state}` über `json()` und den JSON-Initializer.
`id` ist die externe Objektkennung. `text`, `summary`, `translation` sind
bequem zugänglich; weitere Felder werden über `state`/`setStateField` durchgereicht.
Vorhandene Fassungsfelder wie `einfuegungen` und `kiDoc` werden nicht umgedeutet.
`AblageJSON` erhält auch unbekannte Felder, null, große Ganzzahlen und
unterschiedlich normalisierte Unicode-Schlüssel. Sein vorhandener kanonischer
Serializer wird statt des null-entfernenden Bibliotheks-Hashes benutzt.

Quellen: `sync/collect.rs` (`collect`, `apply_page`, `pages_differ`),
`commands/pages.rs`, `library_layout.rs`, `lib/reader/pageState.ts`,
`lib/reader/pasteLog.ts`, `components/settings/tts/TtsSettings.tsx`.
Die Sync-Spezifikation vom 14.09. ist bei den Größen veraltet: Der aktuelle
Rust-Code erlaubt **3 MiB Klartext**, der Hub **4.300.000 Bytes base64-Payload**.
Seiten werden einzeln in der bestehenden Push-Hülle versandt; keine neue Route.
Die niedrigere Bibliotheksgrenze wird für Bücher unverändert gelassen.

**Metadaten bleiben vollständig gerätelokal.** Das Rust-Backend speichert
state.json zwar undurchsichtig, aber `TtsSettings.pageStateJson` baut die
bekannten Felder beim Autosave neu auf; `pageState.withFileLog` rettet daraus
nur `einfuegungen`. Aufnahme-Metadaten im state würden verloren gehen.
Deshalb synchronisiert S1 nur den Seitentext und die übrigen Desktop-Felder,
keine Aufnahme-Metadaten oder Bildnamen. Das ist die im Auftrag zugelassene
Ausweichlösung; Desktop-Änderungen sind für S1 nicht nötig.

`ReadingPageStore` läuft auf dem MainActor unter
`group.de.localvoice.prototype/ReadingPages`. Je Seite liegt ein atomarer
Datensatz `<page-ID>.json` mit Payload, Sync-Basis, Grabstein, geordneten
Aufnahmen und letztem ausdrücklich zusammengesetzten Text. Die lokale Payload
ist als Data im Codable-Datensatz abgelegt; die Hub-Payload bleibt normales JSON.
Bilder liegen unter `Images/<page-ID>/<Bildname>`; `storeImage` schreibt atomar
mit dem vorhandenen Einzelbild-Limit von 40 MiB. Auf iOS gilt Dateischutz bis
zur ersten Entsperrung. Es gibt kein automatisches Löschen von Originalbildern.
Ein Restore oder endgültiges Leeren des Papierkorbs ist kein Teil von S1.

Fehlerhafte Einzeldateien werden weder überschrieben noch als gelöscht gesendet.
Lesbare Seiten bleiben verwendbar. Ein fehlgeschlagener Remote-Apply hält den
bestätigten Pull-Cursor zurück, während weitere Objekte und lokale Uploads
verarbeitet werden. Der neue einmalige Voll-Pull holt früher ignorierte Seiten.
Der MainActor serialisiert Zugriffe innerhalb der App; es gibt keine zusätzliche
prozessübergreifende Schreibsperre. Die Share-Extension schreibt weiter nur
in den bisherigen Eingang, nicht direkt in den Seitenspeicher.

Konfliktvergleich wie `pages_differ`: Titel, Text, Zusammenfassung, Übersetzung;
URL/Reiter allein erzeugen keine Kopie. Das Original übernimmt den Serverstand,
die ungesicherte lokale Fassung wird `<Titel> (Konflikt von <Gerät>)`.
Eine deterministische neue Kennung verhindert doppelte Kopien nach einem Abbruch
zwischen beiden atomaren Schreibvorgängen. Zusätzlich wird auch bei einem
Remote-Grabstein ungesicherter lokaler Text als Kopie gerettet. Gelöschte Originale
und ihre Aufnahmen bleiben archiviert. Bestätigungen merken den gesendeten Hash,
damit eine während der Anfrage entstandene Änderung offen bleibt. Alte Revisionen
und Echos werden ignoriert; Rejections gelten nur für denselben Hash.
Revisionen sind an Hub/Benutzerkennung gebunden und werden bei Kontowechsel
nicht wiederverwendet; lokale Seiten bleiben dabei erhalten.

Für S2: `LibrarySync.readingPageStore()` liefert denselben Speicher wie Sync.
Nach lokalem Speichern muss S2 wie die bisherigen Schreiber den Abgleich anstoßen.
Empfohlen ist der eigene Reiter **Vorlesen zwischen Transkripten und Bibliothek**:
Seiten sind eine eigene Tätigkeit und gehören nicht unter Sprechen.
Aufnahmetexte werden mit **einer Leerzeile (`\n\n`)** verbunden; leere OCR-Texte
tragen nichts bei. Kein automatischer Neuaufbau überschreibt einen Editorstand.
`composedText` merkt den letzten übernommenen Zusammenbau, `textEdited` eine
bearbeitete Aufnahme, `pendingRecognition` ein noch nicht übernommenes OCR-Ergebnis.
`requiresCompositionChoice` signalisiert eine notwendige Wahl. S2 bietet dann
Ersetzen/Anhängen/Abbrechen an und übernimmt Text und Metadaten ausdrücklich.

## Tests und ausstehendes Mac-Tor

Lokal ausgeführt:

```text
python -B apps/apple/scripts/test_reading_page_project.py
2 Tests OK, einschließlich beider Mutationen (je eine Quelle entfernt).
git diff --check
```

Die Projektprüfung wertet die tatsächliche Quellenauswahl des Generators aus,
ohne Projekt-/Plist-Dateien zu schreiben. Sie prüft iPhone-Zuordnung und
Watch-Ausschluss. Sie ist kein Ersatz für Swift-Verhaltenstests.

Auf dem Mac (durch den Folge-Thread; hier nicht ausgeführt):

```sh
swift test --package-path apps/apple/VoiceCore
python3 apps/apple/scripts/generate_project.py
# Danach iPhone-Simulator-Build gemäß apps/apple/README.md.
```

`ReadingPageTests.swift` deckt Desktop-Rundreise inklusive Fassungen und
unbekannter Werte, null/fehlend, Pfade/Größen, vorhandene Verschlüsselung und
AAD-Gegenprobe, zwei Geräte, Konflikt/Echo/Neustart, späte Bestätigung,
Kontowechsel, Grabstein/Replays, Absturz zwischen Kopie und Original,
Schreibfehler, defekte Einzeldatei, lokale Bilder/Metadaten, Kompositionswahl,
Rejections und übergroße Seiten ab. Die Tests enthalten jeweils Gegenfälle
(z. B. Textänderung erzeugt Konflikt, Reiteränderung allein nicht).

Zusätzlich auf dem Mac als echte Rot-Gegenproben zu prüfen (nur in isolierter
Testkopie, keine laufenden Builds im geteilten Baum): unbekannte JSON-Felder
fallen lassen; Konfliktkopie auslassen; Revisionsschutz auslassen;
Bestätigung auf den aktuellen statt den gesendeten Hash setzen; Metadaten
beim Remote-Apply leeren; Ablehnung/Größenprüfung auslassen. Die jeweils
zugehörigen Tests müssen rot werden; anschließend unveränderte Suite grün.
Diese Mutationsläufe sind ausdrücklich noch offen.

Nicht geprüft: Swift-6-Kompilierung/Actor-Prüfung, vollständige VoiceCore-Suite,
iPhone-/Watch-Build, reale Hub-Kommunikation, Gerät mit App-Gruppe und Dateischutz,
Stromausfall-Dauerhaftigkeit. Der Xcode-Projektgenerator wurde hier nicht
schreibend ausgeführt; die neue Quellenliste wird am Mac übernommen.

## Review-Fixrunde auf 44968cb (25.11.2026)

- Der lokale Datensatz erhält optional `imagesID`. Ohne das Feld gilt weiterhin
  die eigene Seitenkennung; alte Datensätze bleiben lesbar. Konfliktkopien
  übernehmen `imagesID ?? id`, auch bei Remote-Grabsteinen und weiteren
  Konfliktgenerationen. `imageURL` löst diesen dauerhaft gespeicherten Verweis
  auf und prüft die Ordnerkennung wie eine Seitenkennung. Bilder werden weiterhin
  nicht gelöscht. Ein späteres Aufräumen muss alle Verweise einschließlich
  Konfliktkopien und Grabsteinen berücksichtigen.
- `save` serialisiert einmal und prüft genau diese Bytes mit demselben
  `ReadingPage`-Parser und derselben 64-MiB-Grenze wie `read`, bevor die bestehende
  Datei ersetzt wird. Die 3-MiB-Sync-Grenze bleibt davon getrennt: Größere,
  lokal lesbare Seiten bleiben speicherbar und werden nicht hochgeladen.
- Neue synthetische Verhaltenstests prüfen Bildzugriff über Konfliktkennungen
  nach Neustart, Remote-Grabstein, weiterer Konfliktgeneration und Löschen sowie
  Ablehnung zu großer/zu tief verschachtelter Seiten. Bei Ablehnung muss die
  vorhandene Datei bytegleich bleiben; eine neue ungültige Seite darf nicht
  angelegt werden. Der bestehende Oversize-Test prüft weiterhin den zulässigen
  Bereich oberhalb der Sync-Grenze.

**Desktop-Folgepaket (keine Änderung in S1):** Unbekannte Felder in
`TtsSettings.pageStateJson` beim Autosave aus dem bisherigen Zustand durchreichen,
statt nur die bekannten Felder neu aufzubauen. Dazu Desktop-Rundreisetests mit
synthetischen unbekannten Aufnahmefeldern über Laden, Bearbeiten und Autosave.
Erst danach ein gemeinsames Schema und den Sync der Aufnahme-Metadaten planen;
Bildübertragung und deren Lebenszyklus sind gesondert zu klären. S1 behält
Aufnahme-Metadaten und Bildordner-Verweise ausschließlich im lokalen Datensatz.

Die optionale Extraktion der LibrarySync-Verzweigungen bleibt offen. Für die
wenigen Bedingungen wurden in dieser Fixrunde keine zusätzlichen öffentlichen
VoiceCore-Schnittstellen eingeführt; Cursor-Rückhaltung und `superseded` sind
weiterhin nicht ohne iPhone-Ziel als Verhalten getestet.

Lokal erneut geprüft: `python -B apps/apple/scripts/test_reading_page_project.py`
(2 Tests einschließlich beider Gegenproben), `git diff --check` und LF der
geänderten Dateien. **Keine Swift-Tests, Builds oder Swift-Mutationen ausgeführt.**
Die neuen Tests wurden vor den Korrekturen geschrieben; ein beobachtetes
Swift-Rot/Grün-Ergebnis steht ausdrücklich aus.

Übergabe an den Mac-Thread, jeweils in isolierter Testkopie:

1. Unveränderte VoiceCore-Suite mit `swift test --package-path apps/apple/VoiceCore`.
2. Gegenprobe Bilder: Nur `saved.imagesID = record.imagesID ?? id` entfernen.
   `ReadingPageStoreTests.testConflictImagesSurviveRestartAndDeletionIncludingRemoteTombstone`
   muss beim Bildzugriff über die Konfliktkennung scheitern (beide Remote-Fälle).
   Zusätzlich nur auf `saved.imagesID = id` ändern: Die weitere Konfliktgeneration
   muss den verlorenen ursprünglichen Ordnerverweis erkennen.
3. Gegenprobe Speichern: Nur den neuen `ReadingPage`-Prüfaufruf in `save` entfernen.
   `testSaveRejectsUnreadableSizeWithoutReplacingRecord` und
   `testSaveRejectsUnreadableNestingWithoutReplacingOrCreatingRecord` müssen rot
   werden (fehlende Ablehnung, ersetzte/angelegte Datei). Danach unveränderte
   Suite erneut grün prüfen. Der Größentest verwendet synthetisch etwas mehr
   als 64 MiB Text und benötigt entsprechend Arbeitsspeicher.

Weiterhin nicht geprüft: Swift-6-/Actor-Kompilierung, iPhone-/Watch-Build,
reales Gerät/App-Gruppe/Dateischutz und Hub-Kommunikation. Keine Git-Mutation,
Netzwerkzugriffe, Installationen oder Eingriffe in laufende Programme.
