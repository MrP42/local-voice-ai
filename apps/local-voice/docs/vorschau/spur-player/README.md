# Spur-Player im Bedienfeld als Einzeiler (Patrick 05.11.2026)

Vorher (`vorher/`, main de0fd40) und nachher (`nachher/`, dieser Branch), gleiche
Testdaten (Aufnahme mit Spur „Ich“ und „Gegenseite“, 35:14, Ich-Spur bei 12:34),
Playwright ohne Tauri-Fensterknöpfe. `spuren-*` ist der Ausschnitt der beiden
Spuren, `seite-*` die ganze Seite.

| | Vorher | Nachher |
|---|---|---|
| Aufbau | Zwei Zeilen je Spur: Knöpfe und Zeit oben, Zeitstrahl allein darunter über die volle Breite | Eine Zeile je Spur: Beschriftung · −10 s ▶ +30 s · Zeitstrahl · Zeit · Tempo |
| Beschriftung | 64 px, „Gegense…“ abgeschnitten | 66 px, halbfett, „Gegenseite“ passt |
| Zeitstrahl | Browser-Standardgriff, 4 px Höhe als Trefferfläche | Fluent-Slider: 4-px-Schiene, Brand-Füllung, Griff mit Ring; Trefferfläche 24 px |
| Zeit | `0:00/35:14` | `12:34 / 35:14`, Gesamtdauer gedämpft, Platz für die Laufzeit reserviert (beide Zeitstrahle gleich lang, kein Springen) |
| Tempo | umrandeter Kasten | schlichter Textschalter 32×24 px, Akzentfläche nur bei Tempo ≠ 1× |
| Höhe beider Spuren | ca. 96 px | 74 px |

Schmale Spalte (Container ≤ 380 px, z. B. 390-px-Fenster): die Sprungknöpfe
entfallen, der Zeitstrahl behält seine Länge. Unter 280 px entfällt zusätzlich
die Gesamtdauer. Die Zeile bricht nie um.

Tastatur: Tab erreicht Sprungknöpfe, Play, Zeitstrahl und Tempo. Auf dem
Zeitstrahl springen ←/→ (↓/↑) 5 s, Bild↑/Bild↓ 30 s, Pos1/Ende an Anfang und
Ende (vorher 0,01 s je Pfeiltaste). Der Zeitstrahl nennt seinen Wert als
„12:34 von 35:14“, jede Spur ist eine Gruppe „Wiedergabe Ich“ bzw.
„Wiedergabe Gegenseite“.

Trefferflächen gemessen: `nachher/trefferflaechen-*.txt` (alle ≥ 24 px, Play 28 px).
Bewusst 24 px statt 44 px auch bei 390 px: Patrick wollte die Zeile kompakt;
24 px ist das WCAG-2.2-AA-Mindestmaß (2.5.8).

Zustände (`nachher/zustand-*`, 1440 hell/dunkel): Tastaturfokus auf Zeitstrahl
und Tempo, Hover auf Tempo, Tempo 1,5× (Akzentfläche).

Kontrast dunkel (gemessen am echten Farbpaar): Griff-Füllung #115EA3 auf
#292929 hätte nur ≈ 2,2 : 1, deshalb trägt der Griff einen äußeren Ring
(Vordergrund 60 % ≈ #A9A9A9) mit ≈ 6,2 : 1 gegen die Fläche – über der
3:1-Grenze für Nichttextkontrast.

Test: `tests/meeting-player.spec.ts` prüft Bild↑ 30 s, → 5 s, ← 5 s, Ende,
Pos1 und dass Alt+→ nicht springt.
