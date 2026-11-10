# Buchimport-Fixtures (lva#149)

Synthetische PDFs, erzeugt mit `erzeugen.py` (reportlab, nur für die Fixtures nötig):

| Datei | Prüft |
|---|---|
| `kurzgeschichte-kopfzeile.pdf` | Kopfzeile mit mitlaufender Seitenzahl auf kurzen PDFs, größerer Titel, „erst am / 23. kommt.“ |
| `kapitel-schriftgroesse.pdf` | Kapitel nur an der Schriftgröße erkennbar, Absatzabstände |
| `kapitel-lesezeichen.pdf` | Kapitel aus den Lesezeichen (Outline) |

Echte Bücher liegen nicht im Repo; `korpus_laden.py` lädt sie, der Test
`pdf_text::tests::korpus` (ignoriert) prüft sie.
