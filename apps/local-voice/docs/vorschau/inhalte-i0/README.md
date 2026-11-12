# Vorschau Inhalte-Modul (Stufe I0, Fassung 2)

Konzept: `docs/superpowers/specs/2026-11-12-inhalte-modul.md`. Statische HTML-Vorschau, keine App-Logik.

Aufruf: `vorschau.html?s=k|o|r|z` (Kurzfassung, Lesefassung, Routine-Plan, Zustände), `&t=dunkel`, `&toast=1`
(Meldung nach dem Einfügen), `&baum=1` (390 px: Baum als eigene Ebene), `&blatt=1` (Quelle und Datenschutz als
Blatt), `&beleg=b4` (Sprung auf einen Beleg mit kurzer Markierung des Abschnitts).

| Bild | Zeigt |
|---|---|
| `01-kurzfassung-*` | Baum; Kurzfassung mit Relevanz, Fristen/Terminen, Kernaussagen, Zahlen, Betroffenen – jede Aussage mit Beleg-Knopf (¶n) auf einen Abschnitt der Lesefassung; Quelle aus dem Abruf; rechts Quelle und Datenschutz |
| `02-lesefassung-bereinigt-*` | Lesefassung mit Abschnittskennungen, Liste der Bereinigungen, Rohfassung wiederherstellen/kopieren, Meldung im Fluss unter den Reitern |
| `03-routine-plan-*` | Routine als Plan des Trockenlaufs: Schritt, Parameter, Recht (Erlaubt/Fragen), Bedingung |
| `04-baum-390-*` | Smartphone: Baum als eigene Ebene, Aufnahme mit Protokoll, Routinen mit Zustand und „Aussortiert“ |
| `05-zustaende-*` | Fehler-, Leer- und Wartezustände mit Text und Folgeaktionen (Konzept §9) |
| `06-quelle-blatt-*` | unter 1280 px: „Quelle und Datenschutz“ als Blatt (Knopf im Dokumentkopf) |
| `07-beleg-sprung-1440-*` | Klick auf Beleg ¶4: Lesefassung, Abschnitt markiert |

Die Markierung in 07 zeigt den Belegsprung, keine Vorlesehervorhebung. Reiter haben `aria-controls` und
Pfeiltasten; Belege sind Knöpfe. Baumsteuerung per Tastatur ist beschrieben (§2.1), in der statischen Vorschau nicht
umgesetzt. Breiten 390/768/1440 (2560 nur Kurzfassung), hell und dunkel; keine horizontale Body-Scrollleiste
(geprüft). Alle Inhalte synthetisch (`example.org`).
