# lva#844 – Designentwurf „Planung“ (Begründung)

Vorschau: `844-entwurf.html` (eigenständig, keine externen Ressourcen, nur example.org-Daten).
Bedienung: Register der Planung anklicken oder Pfeiltasten; „Neue Planung aus Vorlage“ (Strg+N) öffnet die Vorlagenwahl;
„…“ im Kopf öffnet das Überlaufmenü; Thema oben rechts. Zustände per Adresse:
`?thema=hell|dunkel&ansicht=planen|termin|durchfuehren|protokoll&dialog=vorlagen&suche=fix&menue=1&combo=schae`.

Bilder (deviceScaleFactor 1): `844-<ansicht>-<1440|2560>-<hell|dunkel>.png` für planen, termin, durchfuehren,
protokoll, vorlagen; dazu `844-vorlagen-suche-1440-dunkel`, `844-combobox-1440-hell`, `844-menue-1440-dunkel`,
`844-planen-lang-1440-hell` (ganze Seite) und `844-planen-1280-hell`.

Hinweis zur Vorschau: „Durchführen“ und „Protokoll“ zeigen bewusst einen späteren Zustand derselben Planung, damit
alle Bereiche an einem Beispiel zu sehen sind. Die Bilder entstanden unter Linux ohne Segoe UI (Ersatzschrift Inter);
in Windows laufen Texte etwas schmaler.

## 1. Struktur

```
Seitenkopf: Planung + Beschreibung · [Mit Kalender abgleichen] [Neue Planung aus Vorlage | ▾]
┌ Liste (312–400 px) ─────────┬ Planung ───────────────────────────────────────────────────────────┐
│ Suche (Teilwort, Strg+F)    │ INSEL (einzige je Ansicht, dunkel in beiden Themen, feines Raster) │
│ Aktiv · Archiv · Alle       │   Herkunft „Aus Vorlage …“ · Titel · Statuschips · Aktionen · „…“   │
│ ─ Diese Woche ─             │   Fakten: Zeit · Dauer · Teilnehmende · Online · Klasse · Transkr. │
│ ◔ Titel                     │   Phasenleiste = Register: ① Planen ② Termin & Zusagen ③ Durch-   │
│   Datum ▫ Webex  [Chip] 4/8 │   führen ④ Protokoll – je mit Stand (fertig/läuft/Achtung/offen)  │
│ ─ Nächste Woche ─ …         ├────────────────────────────────────────────────────────────────────┤
│ ─ Ohne Termin ─ …           │ Inhalt des gewählten Registers (Karten, Container-Queries)         │
└─────────────────────────────┴────────────────────────────────────────────────────────────────────┘
```

- **Liste:** Zeilen ohne Einzelkarten (Haarlinien), Gruppen nach Zeitraum, je Planung ein **Phasenring** (4 Bögen =
  4 Bereiche; orange = Bereich mit offener Störung) mit Textalternative, Statuschip mit Symbol, Zusagen „4/8“.
- **Kopf der Planung (Insel):** Das moderne Element der Website (dunkler Träger, feines 24-px-Raster mit weicher Maske,
  leichter Navy-Verlauf) – im Werkzeug nur **einmal je Ansicht**, kein Glow, kein Blur, keine Bewegung. Die Aktionen
  wechseln mit dem Bereich (Planen: Übergeben + Speichern; Termin: Speicherstand; Durchführen: „Aufnahme läuft“;
  Protokoll: Exportieren). Seltenes liegt im „…“-Menü (Als Vorlage speichern, Duplizieren, Export, Outlook öffnen,
  Webex absagen, Löschen).
- **Phasenleiste:** gleichzeitig Fortschritt und Untermenü (TabList, ein Tab-Stopp, Pfeiltasten/Pos1/Ende).
- **Planen:** bis 860 px Blattbreite einspaltig; 860–1499 px zwei Spalten (Ziel → Agenda → Rollen → Unterlagen links,
  Seitenleiste rechts); ab 1500 px drei Spalten (Ziel+Rollen | Agenda+Unterlagen | Seitenleiste). Seitenleiste mit
  **Aufklappkarten**: Prüfung (Zähler als Chips), Einladung (Designprofil, Online-Besprechung, Transkription,
  eingeklappte Vorschau), Einwilligungen (eingeklappt, „0 von 8 erfasst“).
- **Termin & Zusagen:** InfoBar bei Verschiebung mit „Webex nachziehen“ (die eine Primäraktion) und „Nur bestätigen“;
  Zusagen-Karte mit Filtern samt Zählern, Verteilungsbalken, Tabelle (Name, „extern“, Rolle, Antwort, zuletzt
  angeschrieben), Aktionen „Erinnerung an 3 …“ (Split: keine Antwort / mit Vorbehalt) und „Vorab-Info …“, Quellhinweis
  statt geschätzter Werte; rechts Termin (Datumskachel, Online-Stand mit altem Zeitpunkt durchgestrichen, Beitritt,
  Abgleich, Kennung) und Verlauf der Aussendungen (ohne Inhalte).
- **Durchführen:** aktueller Punkt groß mit Restzeit-Ring (Mono-Ziffern), Zurück / Weiter / Anhalten; Zeitbox-Spur
  proportional mit Überzug (rot schraffiert) und Uhrzeitskala; Plan/Ist-Tabelle; rechts Anwesende mit Einwilligung,
  Person hinzufügen, Notiz zum Punkt (als Beschluss/Aufgabe markieren), „Besprechung beenden“.
- **Protokoll:** Ziel erreicht als Segmentauswahl (Ja/Teilweise/Nein, ohne Vorschlag des Modells), Protokoll nach
  Agenda mit Beschluss-Hervorhebung und Aufgaben; rechts Schrittliste Erzeugt → Freigeben → Entwurf an Teilnehmende,
  „Protokoll freigeben“ als Primäraktion, Entwurf gesperrt mit Begründung; Grundlage (Audio-Löschung, Einwilligung).
- **Vorlagenwahl (Dialog):** Suchfeld (Teilwort, auch in Stichworten, ä/ae-tolerant, Treffer unterstrichen), Gruppen
  Alle / Zuletzt verwendet / Eigene / Mitgeliefert mit Zählern, Liste mit Symbolkachel, Dauer, Punktzahl,
  Transkription und Schloss für mitgelieferte; rechts Vorschau (Ziel-Muster, Zeitband, Agenda, Rollen-Platzhalter,
  Einladung). Fuß: Vorlagen verwalten, Importieren · Planung anlegen (primär) · Leer beginnen · Abbrechen.
- **#838:** Mehrzeilige Felder nutzen den nativen Greifer unten rechts (`resize: vertical`), keine eigene Position.

## 2. Komponenten und Fluent-Entsprechung

| Entwurf | Fluent 2 / bestehend | Anmerkung |
|---|---|---|
| Seitenkopf mit Befehlen | `PageShell` (vorhanden) | Primär: SplitButton „Neue Planung aus Vorlage“ |
| SplitButton + Menü | `SplitButton`, `Menu`/`MenuList`/`MenuItem` | Menüpunkte mit Untertext (`secondaryContent`) |
| Suche in Liste | `SearchBox` | Filter blendet aus, Zähler bleibt |
| Aktiv/Archiv/Alle | `TabList size="small"` (vorhanden `plg-filter`) | Zähler als Teil des Labels |
| Listenzeile | eigene Zeile, `aria-current` (vorhanden) | Auswahl-Pill links wie Navigation |
| Phasenring | eigenes SVG (16 Zeilen) | Text im `title` und `sr-only` |
| Statuschip | `Badge appearance="tint" shape="rounded"` (vorhanden `Statusmarke`) | immer Symbol + Text |
| Insel-Kopf | eigene Fläche, Tokens lokal überschrieben | einzige Abweichung von reinem Fluent, s. u. |
| Phasenleiste | `TabList` mit eigenem Tab-Inhalt | `selectedValue` immer gesteuert |
| Karte | `Card` bzw. `plg-karte` | Radius 8, shadow4, Fläche Background1 auf Background3 |
| Aufklappkarte | `Accordion`/`AccordionItem` | Zusammenfassung zeigt Zähler |
| Felder | `Field` + `Input`/`Textarea`/`Dropdown` | Label oben, Hilfe unten |
| Art des Ziels | `Dropdown` mit Buchstabensprung (#839) | in Vorschau funktionsfähig |
| Rollen, Verantwortlich | `Combobox` mit Gruppen (#839) | Teilwort, ä/ae-Toleranz, `<mark>` |
| Dauer-Schnellwahl | `ToggleButton` | 30/60/90 |
| Online-Besprechung, Ziel erreicht | `RadioGroup layout="horizontal"` als Segmente | |
| Zeitband, Zeitbox-Spur | eigenes Flex-Band | Werte stehen zusätzlich in Text/Tabelle |
| Restzeit-Ring | eigenes SVG, `role="timer"` | Bewegung nur Fortschritt, keine Animation |
| InfoBar | `MessageBar intent="warning|success"` + `MessageBarActions` | |
| Zusagen-Filter | `ToggleButton shape="circular"` in `Toolbar` | |
| Tabelle | `Table` (oder `DataGrid` später) | |
| Datumskachel, Verlauf | eigene kleine Bausteine | |
| Vorlagenwahl | `Dialog` (breit) + `Combobox`-Suche + `Listbox` | Fokus beim Öffnen in die Suche, Esc schließt |
| Symbolkachel (Vorlagen) | „Featured Icon“ aus dem Designsystem | dunkler Träger, farbiges Lucide-Symbol |
| Symbole | lucide-react (vorhanden) | 16 px in Karten/Menüs, 20 px in Navigation |

**Abweichung Insel:** Fluent kennt keine dunkle Kopffläche im hellen Thema. Sie ist der gewünschte „moderne“ Anteil
nach Art der Website, beschränkt auf eine Fläche je Ansicht, ohne Glow/Blur/Animation. Primärknopf darauf bleibt
Fluent-Blau mit Weiß. Wer das nicht will: Insel per Klasse abschaltbar, dann Kopf auf Background1 (keine weitere
Änderung nötig, weil die Insel nur Tokens lokal überschreibt).

## 3. Kontrast (WCAG 2.2 AA, gerechnet mit der WCAG-Formel)

Fluent-Werte aus `webLightTheme`/`webDarkTheme`. Text braucht 4,5:1, Bedienrahmen/Grafik 3:1.

| Paar | Hell | Dunkel |
|---|---|---|
| Text fg1 auf Karte | #242424/#FFF 15,52 | #FFF/#292929 14,55 |
| Text fg2 auf Karte | 10,05 | 10,01 |
| Hilfetext fg3 auf Karte | #616161 6,19 | #ADADAD 6,48 |
| Hilfetext fg3 auf Basis | 5,68 | 8,21 |
| Platzhalter fg4 | #707070 4,95 | #999 5,11 |
| Weiß auf Primär | #0F6CBD 5,38 | #115EA3 6,66 |
| Link (brand-fg) auf Karte | 5,38 | #479EF5 5,18 |
| Chip info | 5,95 | 6,62 |
| Chip zugesagt/ok | 5,89 | 10,05 |
| Chip warn (Verschoben, Vorbehalt) | #BC4B09/#FFF9F5 4,85 | 6,97 |
| Chip abgesagt/Fehler | 6,55 | 6,20 |
| Text auf InfoBar warn | 14,87 | 14,20 |
| Feldrand unten (strokeAccessible) | 6,19 | 6,48 |
| Auswahl-Pill auf gewählter Zeile | 4,52 | 4,18 |
| Fokusring (fg1, 2 px) | 15,52 | 14,55 |
| Balken zugesagt / abgesagt / Vorbehalt | 5,37 / 6,07 / 3,96 | 5,34 / 4,16 / 9,02 |

Insel (gleich in beiden Themen, gemessen am helleren Ende des Verlaufs #13263C bzw. #0E2236):

| Paar | Wert |
|---|---|
| Titel Weiß | 15,33 |
| Fakten #C9CED6 | 9,69 |
| Herkunft/Phaseninfo #A3A9B3 | 6,48 (dunkles Thema 6,83) |
| Akzent #62ABF5 (Link, Phasenbalken, Punkt) | 6,32 (6,66) |
| Phaseninfo „verschoben“ #FBB48A | 8,76 |
| Chip warn / info auf Tönung | 7,69 / 6,33 |
| Weiß auf Primär | 5,38 |

Grenzfälle, bewusst: (1) Feldrand ringsum `stroke1` hat 1,53 (hell) bzw. 2,53 (dunkel); erkennbar ist das Feld über die
untere Kante `strokeAccessible` (≥ 6:1), wie in Fluent. (2) Die Insel hebt sich im dunklen Thema kaum von der Basis ab
(1,02); sie ist kein Bedienelement, getrennt wird sie über Rand, Navy-Verlauf und Raster. (3) Die Farbe im
Verteilungsbalken ist nie allein Träger: dieselben Zahlen stehen mit Symbol und Text in den Filtern.

## 4. Umsetzung in Teilen (Reihenfolge)

Jedes Paket hält alle vorhandenen `data-testid` und Planungs-Specs grün; Akzeptanz jeweils
`pnpm test:playwright tests/planung` (alle `tests/planung*.spec.ts`) → grün, dazu Vorher/Nachher-Bilder hell/dunkel bei 1440 und 2560.

1. **U1 Rahmen: Insel-Kopf + Phasenleiste** (`PlanungDetail.tsx`, `planung.css`). Bestehende Inhalte nur auf die vier
   Register verteilen (Planen: Formular + Regelhinweise + Vorschau + Einwilligungen; Termin: `TerminKarte`;
   Durchführen: Start/Lauf/Zeitboxen; Protokoll: `ProtokollKarte`). Aktionen je Register, „…“-Menü mit Löschen.
   Hart kodierte `--plg-*`-Farben durch Fluent-Tokens ersetzen. Keine neue Funktion, größter Sichtgewinn.
2. **U2 Liste:** Suche, Gruppen nach Zeitraum, Phasenring, Statuschip, Zusagen-Zähler (sobald #843 liefert, sonst ohne).
3. **U3 Planen:** Raster mit Container-Queries, Aufklappkarten, Zeitband in der Agenda, Dauer-Schnellwahl, #838.
4. **U4 Durchführen + U5 Protokoll:** Restzeit-Ring und Zeitbox-Spur (Darstellung zu `AgendaZeitboxen`/`LaufPanel`),
   Schrittliste und Ziel-Segment (`ProtokollKarte`). Reine Darstellung, Logik bleibt.
5. **Mit den Schwester-Issues, jeweils in deren Paket:** #839 Combobox (Basis für Art, Rollen, Vorlagensuche) →
   #840 Vorlagenwahl-Dialog + SplitButton + „Als Vorlage speichern“ → #843 Zusagen-Karte, Erinnerung, Vorab-Info,
   Verlauf → #841 Feld „Designprofil“ in der Einladungskarte → #842 Online-Segment und InfoBar „Webex nachziehen“
   (erst nach ADR).

## 5. Offen für das Review

- Insel ja/nein (oder nur im hellen Thema) – Geschmacksfrage für Patrick.
- Seitenleiste in „Planen“ mitlaufend (`position: sticky`)? Nur, wenn sie niedriger als der Inhaltsbereich bleibt;
  sonst wäre ihr unteres Ende nicht erreichbar. Deshalb im Entwurf nicht gesetzt.
- Bei 1280 px wird in der Liste der Zusagen-Zähler ausgeblendet (Container-Query), damit Datum und Chip Platz haben.
- Nicht im Entwurf: Entwurfsdialoge für Erinnerung und Vorab-Info (#843), Zuordnungsdialog beim HTML-Import (#841),
  Vorlagenverwaltung (#840). Sie folgen dem Dialogmuster der Vorlagenwahl.
