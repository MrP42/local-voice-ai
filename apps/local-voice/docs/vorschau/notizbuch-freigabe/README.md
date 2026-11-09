# Vorschau: Notizbuch-Freigabe für den Assistenten (lva#396 N2, PR lva#414)

Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=<ansicht>&t=hell|dunkel`.
Grundlage: Spec lva#398 §3.2 (Freigabe je Abschnitt oder ganzes Notizbuch, Standard leer, eine Sichtbarkeitsregel).
Ort: Gesprächseinstellungen › Wissen (bestehendes Seitenpanel, 380 px; unter 820 px ganzflächig), neuer Bereich
„Notizbuch“ unter den bisherigen Bereichen. Alle Bilder aus EINEM Lauf über dieselbe `vorschau.html`
(`belege.json` trägt deren SHA-256). Maß-Belege je Bild: `scrollX` 0, Panelüberlauf 0, `unter44` leer (jede
sichtbare Bedienfläche; bei Kontrollkästchen zählt die ganze Zeile als Ziel).

Bedienung: Schalter `button role=switch`, Notizbuch als Aufklapper (`aria-expanded`), Abschnitte als echte
Kontrollkästchen in einer `role=group` je Notizbuch. „Ganzes Notizbuch“ setzt alle Abschnitte (gesperrt angezeigt)
und gilt auch für künftige. Zähler `role=status`. Gruppen erscheinen als Präfix („Projekte › …“).

| Bild | Breiten | Zeigt |
|---|---|---|
| `01-freigabe` | 2560, 1440, 768, 390 | Bereich an; Arbeit: 2 von 4 Abschnitten; Assistent: ganzes Notizbuch; Privat zugeklappt (0 von 2); Hinweis zur Sichtbarkeitsregel und Quelle je Antwort |
| `02-aus` | 1440, 390 | Bereich aus: kein Baum, nichts wird gelesen (Auswahl bleibt gespeichert) |
| `03-leer` | 1440, 390 | Bereich an, aber kein Abschnitt gewählt: „der Assistent liest nichts aus dem Notizbuch“ |
| `04-kein-notizbuch` | 1440, 390 | Kein Notizbuch auf dem Gerät (Fehler `Fehlt`) |
| `05-version` | 1440, 390 | Nicht unterstützte Notizbuch-Version: Hinweis, nichts wird gelesen, Auswahl bleibt |
| `06-laden` | 1440, 390 | Gliederung wird gelesen (Platzhalter, `aria-busy="true"`) |
| `07-standard` | 1440, 390 | Aufruf ohne Parameter: Bereich an, leere Auswahl |

Daten nur synthetisch. Die Chat-Spalte links ist nur angedeutet (Antwort mit Quelle `Notizbuch › Abschnitt › Seite`).

## Prüfbefunde Runde 2

- Standard leer: Ohne `s` startet die Vorschau mit leerer Auswahl (Bereich eingeschaltet). Die Demo-Auswahl gibt es nur in
  `?s=freigabe` (sowie `aus` und `version`, wo „deine Auswahl bleibt gespeichert“ sichtbar wird).
- Zustand vollständig im Modell: Kontrollkästchen für Abschnitt, Gruppe und Notizbuch (je mit Change-Handler, Teilauswahl
  als `indeterminate`); `render()` übernimmt jede Änderung, Zähler stimmen.
- Fokus: Nach jedem `render()` (auch Auf-/Zuklappen) wird der Fokus über stabile ids auf dasselbe Element gesetzt.
  Zugeklappt bleibt das `aria-controls`-Ziel im DOM, aber `hidden`.
- Schalter „Ganzes Notizbuch <Name> freigeben“: Name enthält den Notizbuchnamen; die Datenschutzfolge steht sichtbar
  darunter (`aria-describedby`), solange der Schalter aus ist.
- Tabs: Pfeiltasten Links/Rechts, Pos1/Ende mit Roving-Tabindex; Tab und Panel über `id`, `aria-controls`,
  `aria-labelledby` verbunden; `aria-selected` korrekt.
- Zustand „aus“ zeigt „Kein Zugriff; deine Auswahl bleibt gespeichert.“; im Ladezustand `aria-busy="true"`.
- Neuer Beleg `07-standard` (ohne Parameter). Bildhöhen so gewählt, dass der Baum vollständig sichtbar ist.
