# Apple-Oberflächen – Abgleich mit der Desktop-App

Die native Oberfläche übernimmt neben der WAI-Farbpalette jetzt die vorhandene
Desktop-Komponentensprache. Kleine Kopfzeilen, normale Systemschrift und die
kompakte Inhaltsaufteilung aus dem vorherigen Auftrag bleiben erhalten.

| Desktop-Quelle | Native Übernahme |
|---|---|
| `src/styles/theme.css` | Signalgelb, Ink, Text-/Grundflächen und Mittelgrau |
| `components/ui/Button.tsx` | Primäraktion mit 8-Punkte-Radius, Ink auf Gelb, gedrückter Zustand |
| `components/ui/SettingsGroup.tsx` | Neutrale Flächen, feine Mittelgrau-Kontur mit 20 % Deckkraft, kleine Radien |
| `components/icons/LocalVoiceAiLogo.tsx` | Gleiche fünf Wellenbalken und KI-Punkt im 32-Punkte-Koordinatensystem |
| `App.css`, `.mediabar` / `.mbtn` | Runde Wiedergabeglyphen, ein Primärschalter; Gelb auf Ink im hellen und Ink auf Gelb im dunklen Design |

Alle Desktop-Pfade beziehen sich auf `apps/local-voice/`. Das Markenlogo bleibt
klein neben dem Aufnahmetext; es gibt keine große zusätzliche Markenüberschrift.
Runde native Pillen für die Hauptaktion und große weiße Karten wurden durch die
Desktop-Muster ersetzt. Für Navigation, Systemdialoge und Formulare bleiben native
Apple-Komponenten bestehen, wie im Mobile-/Watch-Plan vorgesehen.

Wiedergabeschalter besitzen trotz reiner Symbolansicht deutsche Bedienungshilfe-
Namen. Die Wiedergabeschalter sind mindestens 44 Punkte groß. Die zugrunde liegende Aufnahme-,
Speicher-, Übertragungs- und Verarbeitungslogik wurde nicht verändert.
