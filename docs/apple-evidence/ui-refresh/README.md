# Native Apple UI – Überarbeitung vom 06.09.2026

Die iPhone-App erhält zwei getrennte Bereiche: Sprechen und Verlauf. Ein kompakter
Aufnahmebereich erklärt Start, Sicherung und die Grenze von 30 Sekunden. Status und
Watch-Verbindung bleiben sichtbar. Die letzten fünf Einträge bieten einen kurzen
Einstieg; der vollständige Verlauf ist durchsuchbar. Jede Notiz öffnet eine eigene
Detailansicht mit Transkript, Antwort, Wiedergabe, Originalexport und gegebenenfalls
Wiederaufnahme der Verarbeitung. Modellverwaltung öffnet über die Werkzeugleiste
und besitzt einen ausdrücklichen Fertig-Button.

Die Watch zeigt zuerst Verbindung, Aufnahme und Status, danach die letzte Notiz
und den vollständigen Verlauf. Große native Schaltflächen, Systemschriftgrößen,
SF Symbols und semantische Hintergrundfarben passen sich der Plattform an.
Aufnahme und Speichern unterscheiden sich durch Symbol, Beschriftung und Farbe.
Die WAI-Markenfarben folgen `apps/local-voice/src/styles/theme.css`: Signalgelb
`#FFDD00`, Ink `#111418`, Text `#1F2937` / `#EDEDE7` und Grundflächen
`#F8F9FB` / `#0B0B0C`. Auf gelben Aktionen steht Ink, niemals Weiß. Gelbe Symbole
stehen auf einem Ink-Träger; Textaktionen sind im hellen Design Ink und im dunklen
Design Gelb. Native Apple-Flächen und Schriftstile bleiben gemäß Apple-Plan erhalten.
Transkripte und Antworten sind auf dem iPhone auswählbar.

Die Speicher-, Transport- und Inferenzlogik wurde nicht verändert. Bestätigungen
bezeichnen weiterhin dauerhafte Speicherung, keine Garantie sofortiger Verarbeitung.
Die P2-Grenzen für Antwortqualität, Latenz und physische Akkumessungen bleiben gültig.
Desktop-Code bleibt unverändert. Die UI enthält keine neue Löschfunktion.

## Prüfung

Build-/UI-Testergebnisse und die visuell geprüften Ansichten werden im zugehörigen
Nachweispaket gespeichert. Der Core-/Transport-/Provider-Code bleibt gegenüber
P2 unverändert; dafür wurde keine neue 100er-Latenz- oder Akkuabnahme behauptet. Geprüft werden Aufnahme/Stop/Home auf beiden Plattformen,
Setup-Wiederholung, beschädigter Verlauf, Modellverwaltung und der neue Ablauf
Modellansicht schließen → Verlauf → Notiz → Suche → Aufnahmebereich.

## Normale Schrift und Bedienungshilfen

Die Simulatoren verwenden zur Übergabe wieder die normale Systemschrift. Die
maximale Bedienungshilfe-Schrift wurde ausschließlich zur Prüfung aktiviert und
danach zurückgestellt. Bei dieser ausdrücklich gewählten großen Schrift steht die
Aufnahme zuerst; dekorative Inhalte entfallen und Texte bleiben skalierbar.
Das große Testbild ist keine Voreinstellung der App.

## Designquelle

WAI-Abgleich: `apps/local-voice/src/styles/theme.css`, `docs/ROADMAP.md` (E4) und
`docs/superpowers/plans/2026-09-05-local-voice-mobile-first-watch.md`.
Alle sechs übernommenen Farbwerte wurden automatisch gegen die vorhandenen
Desktop-Tokens verglichen; `wai-token-check.json` dokumentiert den Abgleich.
Der frühere Grün/Mint-Entwurf ist verworfen und entspricht nicht dem finalen Stand.

## Platznutzung

„Local Voice“ erscheint auf dem iPhone nur noch klein in der Navigationsleiste;
auf der Watch entfällt der Marken-Schriftzug. Der Verlauf nutzt ebenfalls eine
kompakte Navigationsleiste. Das Startsymbol ist 44 statt 76 Punkte groß, steht
neben dem Text und erhält keine eigene große Zeile. Engere Abstände und einzeilige
Verlaufsvorschauen zeigen mehr Einträge. Die Detailansicht enthält weiterhin den
vollständigen Text. Die Hauptaktion und die iPhone-Aktionszeile behalten mindestens
44 Punkte Höhe. Große Bedienungshilfe-Schriften werden nicht künstlich begrenzt.

## Abschlussprüfung

Die finale kompakte WAI-Fassung besteht fünf native iPhone-UI-Tests und einen
Watch-UI-Test, einschließlich frischer Builds beider App-Ziele. Geprüfte Bilder
liegen unter `screenshots/`, Build-/Testprotokolle unter `logs/`.
Die große Systemschrift wurde an einem früheren UI-Zwischenstand geprüft;
für die finale kompakte Fassung wird keine vollständige A11y-Abnahme behauptet.
Zur Übergabe laufen beide Simulatoren mit normaler Schrift; das iPhone ist wieder
im hellen Erscheinungsbild. Die gespeicherten Bildschirmbilder zeigen reale
Testverläufe; Transkript-/Antwortqualität ist weiterhin die dokumentierte P2-Grenze.
