# Vorschau P4b: Projektseite mit „Als Kontext“ (lva#365)

Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=<ansicht>&t=hell|dunkel`.
Grundlage: Spec §5.1/§5.2, Plan `docs/plans/2026-11-08_projekte-p4.md`, Backend P4a (lva#382).
44 Bilder, je hell und dunkel. Maß-Belege in `belege.json` (Playwright): `scrollX` und Spaltenüberlauf
überall 0, `unter44` überall leer (Knöpfe, Schalter, Menüpunkte, Reiter, Zeilen).

| Bild | Breiten | Zeigt |
|---|---|---|
| `01-projekt` | 2560, 1440, 768, 390 | Projektseite gruppiert (Bücher, Texte, Aufnahmen, Dateien im Projektordner, Recherchen). Je Eintrag Schalter, Form, Token. Hinweis statt Rohtext bei fehlendem Kurzbuch. PDF/XLSX: Formathinweis, Schalter gesperrt. Automatisch zugeordnete Aufnahme startet aus. Gefundene Datei wird angeboten, nicht still aufgenommen. Rechts Summe, „Nicht im Kontext“, Geltung nur auf diesem Gerät. Ab 999 px Summenleiste unten |
| `02-form` | 1440, 390 | Form (Kurzbuch Vorgabe / Volltext) und Fassung je Mitgliedschaft |
| `03-zugross` | 1440, 390 | Kontext über dem Modellfenster: sichtbarer Stopp, nichts wird gekürzt |
| `04-reset` | 1440, 390 | Kontextauswahl defekt: alles aus, Hinweis |
| `05-hinzufuegen`, `05-dateien` | 1440, 390 / 1440 | App-Inhalte verknüpfen (bewusst zugeordnet = an); Dateien kopieren mit Prüfung, ohne Überschreiben |
| `06-entfernen`, `06-entfernen-app`, `06-menu` | 1440, 390 / 1440 / 1440 | Kopie in den Papierkorb des Projekts; App-Inhalt bleibt, nur die Verknüpfung entfällt |
| `07-leer` | 1440 | Leeres Projekt |
| `08-vorlesen`, `09-gespraech` | 1440, 390 | Gruppe „Projekte“ in den Leisten von Vorlesen und Sprachgespräch. Das Gespräch im Projekt bleibt gesperrt, bis P4c alle KI-Wege umgestellt hat |

Nicht in P4b: Ausgabeziel „Speichert in“ und Mehrfachaktionen (P5), Chat im Projekt (P4c).
