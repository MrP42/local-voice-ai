# Vorschau Inhalte-Modul (Stufe I0, Fassung 3)

Konzept: `docs/superpowers/specs/2026-11-12-inhalte-modul.md`. Statische HTML-Vorschau, keine App-Logik.

Aufruf: `vorschau.html?s=k|o|r|z` (Kurzfassung, Lesefassung, Routine-Plan, Beispielkatalog der Zustände), `&t=dunkel`.
Weitere Parameter:

| Parameter | Wirkung |
|---|---|
| `&toast=1` | Meldung nach dem Einfügen (im Fluss unter den Reitern) |
| `&baum=1` | 390 px: Baum als eigene Ebene |
| `&blatt=1` | Quelle und Datenschutz als Blatt (unter 1280 px; bei 768 px 80 % hoch, unter 768 px Vollbild) |
| `&beleg=b4` | Sprung auf einen Beleg: Abschnitt markiert, Fokus auf dem Abschnitt |
| `&rev=2` | Die Lesefassung ist inzwischen Fassung 2: Kurzfassung „Veraltet“, Beleg ¶4 mit „geändert“; mit `&beleg=b4` öffnet der Beleg die ältere Fassung 1 (Hinweis „Ältere Fassung“, „Block seitdem geändert“, aktueller Text zum Vergleich) |
| `&mehr=1` | Mehr-Menü offen; unter 768 px mit „Lesen“ und „Bearbeiten“ |
| `&quelle=1` | Hinweis „Quelle geändert – Übernehmen/Vergleichen“; zweite Quellfassung im Quelle-Bereich, Lesefassung unverändert |

| Bild | Zeigt |
|---|---|
| `01-kurzfassung-*` | Baum; Kurzfassung mit Relevanz, Fristen/Terminen, Kernaussagen, Zahlen, Betroffenen – jede Aussage mit Beleg-Knopf (¶n) auf einen Abschnitt der Lesefassung; Quelle aus dem Abruf; rechts Quelle und Datenschutz (KI-Freigaben „alle gelten zusammen“) |
| `02-lesefassung-bereinigt-*` | Lesefassung mit Abschnittskennungen, Liste der Bereinigungen, Rohfassung wiederherstellen/kopieren, Meldung im Fluss unter den Reitern |
| `03-routine-plan-*` | Routine als Plan des Trockenlaufs: Schritt, Parameter, Recht (mit Namen des Rechts: „Inhalte anlegen“, „KI nutzen“; `content.relevance` braucht „KI nutzen“ und alle Freigaben aus §6), Bedingung |
| `04-baum-390-*` | Smartphone: Baum als eigene Ebene, Protokoll als verschachteltes Kind der Aufnahme, Routinen mit Zustand und „Aussortiert“ |
| `05-zustaende-*` | **Beispielkatalog** der Fehler-, Leer- und Wartezustände mit Text und Folgeaktionen (Konzept §9); die Zustände schließen sich teils aus |
| `06-quelle-blatt-*` | unter 1280 px: „Quelle und Datenschutz“ als Blatt (Knopf ⓘ im Dokumentkopf) |
| `07-beleg-sprung-1440-*` | Klick auf Beleg ¶4: Lesefassung, Abschnitt markiert (Belegsprung, keine Vorlesehervorhebung) |
| `08-beleg-geaendert-*` | Kurzfassung zu Fassung 1, Lesefassung inzwischen Fassung 2: „Veraltet“, Beleg „¶4 geändert“ |
| `09-beleg-aeltere-fassung-*` | Beleg ¶4 öffnet die Bezugsrevision (Fassung 1) mit „Ältere Fassung“ und „Block seitdem geändert“ |
| `10-mehr-menue-390-*` | Smartphone: Mehr-Menü mit „Lesen“ (Häkchen) und „Bearbeiten“ sowie Versionen, Letzte Einfügungen, Quelle prüfen, Nur Text exportieren |
| `11-quelle-geaendert-*` | Neue Quellfassung getrennt gespeichert, Lesefassung unverändert, Hinweis „Übernehmen/Vergleichen“ |

Breiten 390/768/1440 (2560 nur Kurzfassung), hell und dunkel; keine horizontale Body-Scrollleiste (per Skript
geprüft, auch für alle Zustände bei allen vier Breiten). Alle Inhalte synthetisch (`example.org`).

## Was die Vorschau kann

- Reiter Kurzfassung/Lesefassung (und die Vorschau-Reiter Routine/Zustände) mit `aria-controls` und Pfeiltasten.
- Belege sind Knöpfe; der Sprung setzt den Fokus auf den Zielabschnitt.
- Das Quelle-Blatt ist modal: Fokus wandert hinein, Escape schließt, Fokus kehrt zum Knopf zurück, der Hintergrund
  ist inert.
- Mehr-Menü mit Pfeiltasten und Escape; „Lesen/Bearbeiten“ schaltet die Kennzeichnung im Kopf.

## Statische Einschränkungen (keine App-Logik)

- Die Reiter „Übersetzung“ und „KI-Dokumente“ sind ohne Funktion (`aria-disabled`); „Routine“ und „Zustände“ sind
  reine Vorschau-Reiter und gehören nicht zum Produkt.
- Die Baumsteuerung per Tastatur (Pfeile, Pos1/Ende, `*`, Typeahead) ist in §2.1 beschrieben, hier nicht umgesetzt.
- Suche, Liste, Neu-Menü, Wiederherstellen, Vergleichen, Übernehmen und alle anderen Knöpfe sind Attrappen.
- Der Zustandskatalog (`s=z`) zeigt Zustände nebeneinander, die in der App nie zugleich auftreten.
- Die Zustände Revision/Quelle (`rev`, `quelle`) werden über URL-Parameter gewählt, nicht aus Daten berechnet.
