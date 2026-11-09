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
| `06-laden` | 1440, 390 | Gliederung wird gelesen (Platzhalter) |

Daten nur synthetisch. Die Chat-Spalte links ist nur angedeutet (Antwort mit Quelle `Notizbuch › Abschnitt › Seite`).
