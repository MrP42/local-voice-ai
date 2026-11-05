# Einheits-Player auf dem iPhone: Hören, Mitlesen, Lesen in einem Bildschirm

Stand: 05.11.2026 · Auftrag Patrick (Thread „Einheitlichen Lese- und Hör-Player bauen“)

## Ziel

Ein Player für alles. Der Wechsel zwischen Audio, Text und Hybrid geht mit einem Tipp im selben Bildschirm, ohne Menüweg und ohne Positionsverlust. Beim Hörbuch liest man synchron mit: Satz und Wort sind markiert, der Text scrollt mit.

## Befund vorher (Code main 93c63364)

| Schritt | Heute |
|---|---|
| Buch öffnen | Bibliothek → Buchdetail → **zwei** Knöpfe „Text vorlesen“ oder „Hörbuch hören“ |
| Player | zwei getrennte Vollbild-Ansichten (`LibraryReadAloudView`, `LibraryListenView`) |
| Audio → Text | Player schließen, im Detail den anderen Knopf wählen; die Stelle wird nicht übergeben |
| Mitlesen im Hörbuch | Knopf „Text“ in der zweiten Bedienreihe; markiert nur den **Absatz**, ohne Zeitachse linear geschätzt |
| Wortmarkierung | keine (auch die Systemstimme meldet Wörter, genutzt wurde nur der Satz) |

## Vorbild (REA-Läufe 01.11., `/mnt/project-files/reverse-engineering/`)

- Audible „Read & Listen“: Umschalter **Hören ↔ Lesen & Hören** über dem Cover, wortgenaue Markierung, Darstellung (Schrift, Größe, Thema) im selben Player (A-REA-4, K-REA-3).
- Kindle: unaufdringliches Chrome, Fortschritt unten, Antippen setzt die Stelle.

## Lösung

### Ein Einstieg
Buchdetail hat **einen** Knopf „Weiter“ bzw. „Starten“. Er öffnet den Player mit der zuletzt genutzten Quelle und Ansicht des Buchs (`LibraryPlayback.open`). Hörbuch-Teile in der Liste öffnen denselben Player.

### Drei Ansichten, ein Umschalter (`PlayerSurface`, VoiceCore)
Segment oben unter der Kopfzeile, nur wenn das Buch Text hat:

| Ansicht | Inhalt | Wiedergabe | Bedienleiste |
|---|---|---|---|
| **Hören** | Cover, Titel, Mitwirkende | läuft weiter | volle Leiste |
| **Lesen & Hören** | Text, Satz hell, Wort kräftig, scrollt mit; eigenes Scrollen pausiert 5 s, „Zur aktuellen Stelle“ | läuft weiter | volle Leiste |
| **Lesen** | Text ruhig, Leseposition dezent, kein Mitscrollen; Antippen setzt die Stelle | hält an | schmale Leiste: Fortschritt + „Ab hier hören“ |

„Lesen“ → „Lesen & Hören“ spielt weiter, wenn vorher lief. Die Ansicht wird je Buch gemerkt (`lva.player.surface.<id>`).

### Quelle im selben Player (`PlayerSource`, Menü „Quelle“)
Hörbuch (Sprecher) oder Vorlesestimme. Der Wechsel übergibt die Textstelle: vom Hörbuch zur Stimme ab Beginn des laufenden Satzes (`ReadAlong.handoverChars`), von der Stimme zum Hörbuch über die Zeitachse. Läuft die Wiedergabe, läuft sie an der neuen Quelle weiter. Je Buch gemerkt (`lva.player.source.<id>`). Liegt das Hörbuch nur im Konto, ist die Option sichtbar, aber gesperrt mit dem Hinweis „erst im Buch laden“.

### Mitlesen (`ReadAlong.mark`)
- **Hörbuch mit Zeitachse** (Stützpunkte vom PC, `audio.sync`): Zeichenstelle aus der Abspielzeit, daraus Satz und Wort. Zwischen Stützpunkten anteilig.
- **Hörbuch ohne Zeitachse**: nur Absatz, Hinweis „Absatz nur geschätzt“ (keine falsche Wortgenauigkeit).
- **Systemstimme**: Wort exakt aus `willSpeakRangeOfSpeechString`.
- **Phonon**: Satz (Phonon liest satzweise).
- Markierung nur als Hintergrundfarbe, Schriftschnitt bleibt: kein Umbruchspringen (Lehre aus lva#232).

### Kopfzeile
Schließen · Titel/Untertitel (Kapitel oder Stimme, Zustand) · **Aa** (Darstellung) · **Fragen** (Buch-Gespräch, Symbolknopf) · ⓘ (nur Hörbuch).

## Dateien

| Datei | Inhalt |
|---|---|
| `VoiceCore/Sources/VoiceCore/ReadAlong.swift` | `PlayerSurface`, `PlayerSource`, `ReadingMark`, `ReadAlong` (ohne UI, getestet) |
| `VoiceCore/Tests/VoiceCoreTests/ReadAlongTests.swift` | Ansichten, Quelle, Wortgrenzen, Satz/Wort aus Zeitachse, Übergabestelle |
| `iPhone/Library/BookPlayer.swift` | `BookPlayerScreen`, `SurfaceSwitch`, `ReadAlongText`, `ReadingBar`, `VoiceControls`, `SourceMenu` |
| `iPhone/Library/AudiobookPlayer.swift` | `PlayerCoverArea`, `AudiobookControls` (aus `LibraryListenView` herausgelöst) |
| `iPhone/Library/LibraryPlayback.swift` | `open`, `show`, `switchSource`, `readingMark`, `wordRange`, Satz-Zwischenspeicher |
| `iPhone/Library/LibraryViews.swift` | Buchdetail mit einem Knopf; `FollowText(follow:)`; alte Vorlese-Ansicht entfernt |
| `iPhone/BookTalk/BookTalkSheet.swift` | `BookTalkButton(compact:)` (abgestimmt mit dem Buch-Gespräch-Thread) |

## Grenzen und Folgeschritte

1. **Zeitachse auf dem iPhone selbst** erzeugen (Abgleich wie Desktop `hoersync`, mit SpeechTranscriber), damit Hörbücher ohne PC-Abgleich wortgenau mitlaufen.
2. Eigene TTS-Hörbücher: Zeitmarken beim Rendern am Desktop als `audio.sync` mitschreiben (exakt, ohne Abgleich).
3. „Lesen“ zählt Scrollen noch nicht als gelesen; nur Antippen setzt die Stelle.
4. Antippen der Textmitte blendet Kopf und Leiste noch nicht aus (Kindle-Muster).
5. Desktop: gleiches Muster (Umschalter über dem Inhalt, Symbolknopf „Fragen“ oben rechts).
6. Audible/Kindle: nur rechtlich saubere Wege, siehe `/mnt/project-files/einheits-player/audible-kindle-rechtswege.md`.
