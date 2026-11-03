# Vorschau: Hörbuchplayer auf dem iPhone

Ausbau des Hörbuchplayers der iPhone-Bibliothek (#210): Cover, Kapitel mit
Fortschritt, Inhaltsverzeichnis mit Sprung, Info-Blatt mit den Angaben aus der
M4B/AAC- bzw. MP3-Datei. iPhone 390 × 844 pt, Faktor 2. Farben aus `VoicePalette`.
Erzeugt aus `vorschau.html` (Parameter `theme`, `screen`, `dt`, `touch`).

Beispiel: Fontane, „Effi Briest“ (gemeinfrei), 36 Kapitel, 9:52:42. Cover,
Sprecherin und Verlag sind erfunden.

| Bild | Inhalt |
| --- | --- |
| `01-player-*` | Cover aus der Datei, Titel, „von … · gelesen von …“. Kapitelleiste („Kapitel 8 von 36“, Titel) öffnet das Inhaltsverzeichnis. Regler über das laufende Kapitel mit „noch … im Kapitel“, darunter Buchfortschritt in Prozent, Restzeit und „etwa Seite 74 von 349“. Knöpfe: Kapitel zurück, 15 s zurück, Pause, 15 s vor, Kapitel vor; Tempo, Inhalt, Text/Cover, Schlaftimer. ⓘ oben rechts öffnet die Angaben. `gross` = Dynamic Type etwa 120 % (Fortschritt bricht in zwei Zeilen um), `touch` = antippbare Flächen rot gestrichelt |
| `02-inhalt-*` | Inhaltsverzeichnis als Blatt: öffnet am laufenden Kapitel. Gehörte Kapitel mit Haken, laufendes markiert mit eigenem Fortschritt und Restzeit, folgende mit Nummer und Startzeit, rechts die Dauer. Antippen springt an den Kapitelanfang und spielt. `oben` zeigt die Zusammenfassung: Kapitelzahl, Gesamtdauer, Prozent gehört, Restzeit, „Es folgen noch 28 Kapitel“, Seite |
| `03-info-*` | Angaben: Autor, Sprecher, Reihe, Verlag, Erscheinungsdatum, Genre, Sprache, ISBN; Umfang: Dauer, Kapitel, Seiten (aus der Buchsuche oder geschätzt, dann so beschriftet), Teile, Format/Codec/Bitrate, Größe; Beschreibung mit „Mehr anzeigen“; Copyright, Erstellt mit. Leere Felder entfallen |
| `04-mitlesen-dunkel` | Mit Text: Mitlesen wie bisher, Kapitelleiste und Fortschritt bleiben; „Cover“ schaltet zurück |
| `05-schlaftimer-kapitelende-hell` | Schlaftimer „Ende des Kapitels“ aktiv |

## Woher die Angaben kommen

| Angabe | M4B/M4A/AAC (MP4-Atome) | MP3 (ID3) |
| --- | --- | --- |
| Titel / Album | `©nam` / `©alb` | `TIT2` / `TALB` |
| Autor | `©ART`, `aART` | `TPE1`, `TPE2` |
| Sprecher | `©wrt` (Hörbuch-Konvention), `©nrt` | `TCOM` |
| Reihe | `©grp`, `©mvn` | `TIT1` |
| Verlag, Datum | `©pub`, `©day` | `TPUB`, `TDRC`/`TYER` |
| Beschreibung | `desc`, `ldes`, `©cmt` (längste gewinnt, HTML wird bereinigt) | `COMM` |
| Cover | `covr` | `APIC` |
| Kapitel | Kapitelspur (QuickTime-Chapter / Nero `chpl`) | `CHAP` (soweit AVFoundation sie liefert) |

Ohne Kapitelmarken werden die Teile (Dateien) als Kapitel gezeigt; das Blatt sagt das.
Seiten: Hörbücher haben keine. Angezeigt wird die Seitenzahl aus der Buchsuche am PC,
sonst eine Schätzung aus der Textlänge (1 800 Zeichen je Seite) oder der Hördauer
(1,7 Minuten je Seite), immer mit „etwa“. Sperrbildschirm und Kopfhörer: Cover,
Kapiteltitel, Kapitel vor/zurück.
