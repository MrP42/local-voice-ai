# Belege: Seite „Ablage“ in der App (D1, lva#407)

Aufnahmen der echten React-Seite (Playwright, Chromium, Skalierung 1) mit synthetischen Daten und erfundenen Namen,
Tauri-Befehle `ablage_*` im Test gespielt. Vergleichsbild: die angenommene HTML-Vorschau `../ablage-d1/`.
Basis: Head-Commit `73cca600` plus die Änderungen dieses Commits. Erzeugung:
`ABLAGE_SHOTS=1 pnpm exec playwright test tests/ablage.spec.ts --trace off --workers=1` (Spec: `tests/ablage.spec.ts`).
Maße je Bild stehen in `belege.json` (`scrollBreit` gegen `fenster`, `unter44` nur bei 390 px, Dialoge und Blätter inklusive).

Namensschema `<ansicht>-<breite>-<hell|dunkel>.png`, Breiten 390 und 1440 (Höhe 844 bzw. 900).

| Ansicht | Zeigt |
|---|---|
| `liste` | Ordner „Allgemein“ mit fünf Dateien (docx, xlsx mit Konflikt, eml, md, pdf); 1440: Ordnerbaum, Liste, Details der ersten Datei; 390: Liste mit Kopfleiste |
| `hochladen` | Hochladeliste mit abgelegter Datei und Namenskollision („Als neue Fassung“, „Beide behalten“, „Überspringen“); 390 als Blatt |
| `ordner` | Dialog „Ordnereinstellungen“ für „Verträge“ (KI-Freigabe, Datenklasse, Nur dieses Gerät) |
| `suche` | Namensfilter „vertrag“ im Ordner „Verträge“ (3 von 4 Dateien) |
| `papierkorb` | Zwei Einträge (Datei, Ordner) mit Resttagen und Ursprung |
| `leer` | Leerer Ordner mit Hinweis und „Auswählen“ |
| `ordnerwahl` (nur 390) | Ordnerbaum als Blatt (Dialog, aria-modal) über den Kopfknopf „Ordner wählen“ |

Ordner der Daten: „Allgemein“ (KI aus), „Verträge“ (KI an, vertraulich), „Projekte“ (gesperrt) mit „Brücke Nord“ (erbt gesperrt, nur dieses Gerät).

## Gemessene Ergebnisse (38 Prüfungen, alle grün)

- Seitenüberschrift: genau ein `h1` „Ablage“ aus der Shell; Ordnerbaum und Listenzeilen sichtbar; Namensfilter zählt 3 von 4, „zzz“ zeigt „Keine Datei passt zu dieser Suche.“
- Kein waagerechtes Rollen (`scrollWidth <= innerWidth`) bei 390 und 1440 px in Liste, Papierkorb, Hochladen, Dialog; alle 26 Bilder: `scrollBreit == fenster`.
- Bedienflächen bei 390 px: alle sichtbaren Steuerelemente der Ablage (Seite, Hochladeblatt, Ordnerblatt, Einstellungsdialog) mindestens 44 × 44 px (`unter44` in allen Bildern leer). Vor der Änderung waren Symbolknöpfe 32 × 32 und der Knopf „Dateien“ 32 px hoch.
- Shell außerhalb der Ablage unter 44 px (nur berichtet, nicht Teil dieser Seite): Menü öffnen 32×32, Über diese Seite 28×28, Suchfeld 110×30, Statuszeile (Modell, Mikrofon, Anruf, Version) 22 px hoch.
- Escape schließt und gibt den Fokus an den Auslöser zurück: Ordnereinstellungen, Neuer Ordner, Papierkorb leeren (390 und 1440), Dateiblatt, Hochladeblatt, Ordnerblatt (390). Beim Schließen der Desktop-Details geht der Fokus an die Dateizeile zurück.
- Ordnerblatt unter 1024 px: Tab bleibt im Dialog (12 Tabs), Wahl per Enter öffnet den Ordner und schließt das Blatt.
- Keine rohen Übersetzungsschlüssel in Liste und Dialog.

## Gefundene und behobene Mängel

- Unter 1024 px fehlte jede Ordnerauswahl: neuer Kopfknopf „Ordner wählen“ öffnet den Baum als Blatt.
- Bedienflächen auf schmalen Breiten kleiner als 44 px (Symbolknöpfe, Dialogfußzeilen): Mindestgröße in `ablage.css`.
- Fokus kehrte nach Dateiblatt, Hochladeblatt und Ordnereinstellungen nicht zum Auslöser zurück (Blatt wurde beim Schließen ausgehängt bzw. Auslöser nicht gemerkt).

## Offen (nicht behoben)

- 1440 px mit geöffneten Details: die Spalte „Stand“ der Liste ist schmal, „Konflikt“ wird abgeschnitten (`liste-1440-*`).
- Kontrast nicht gesondert gemessen (der Seite liegen die Tokens der Shell zugrunde, `a11y-contrast.spec.ts` deckt die Ablage nicht ab).
