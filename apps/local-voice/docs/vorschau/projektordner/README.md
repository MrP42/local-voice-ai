# Vorschau: Buch-Ordner im Vorlesen und appweite Projekte

Spec: `docs/superpowers/specs/2026-11-07-buch-ordner-und-projekte.md` (Entscheidungen E1–E5 in Abschnitt 7).
Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=buch|alle|projekt|ablage&t=hell|dunkel&ebene=ordner|liste|reader`.

| Bild | Zeigt |
|---|---|
| `01-buchordner-*` | Vorlesen nach Aufnahmen-Muster: Ordnerleiste (Bücher je Buch, Texte, Projekte), Inhaltsliste des Buch-Ordners, Reader, Kapitel rechts. 390: drei Ebenen (`-ordner`, `-liste`, `-reader`) |
| `02-alle-*` | Ordner „Alle“: Bücher und lose Texte getrennt |
| `03-projekt-*` | Projektseite: Einträge gruppiert, Schalter „Als Kontext“ mit Token, KI im Projekt, Ziel für Ergebnisse |
| `04-ablage-*` | Buchdetail „Dateien im Buch-Ordner“ (Verzeichnisbaum) |
| `05-migration-*` | Einmaliger Dialog „Bibliothek neu ordnen“ mit Sicherung und Rückfall |

Kopfzeile rechts: aktives Projekt als Ziel („Speichert in: Finanzwissen“). Breiten 390/768/1440/2560 hell und dunkel;
geprüft: keine waagrechte Scrollleiste, alle Bedienziele ≥ 44 px bei 390 px.
Das Symbol des Sprachgesprächs ist hier bereits eine Sprechblase statt eines Telefons (Umsetzung im Thread für die kleinen UI-Fehler).
