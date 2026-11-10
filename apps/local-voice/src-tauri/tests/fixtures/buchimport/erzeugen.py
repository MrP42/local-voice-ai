"""Synthetische Test-PDFs für den Buchimport (lva#149): Kopfzeile mit
mitlaufender Seitenzahl, größerer Titel, Lesezeichen."""
import sys
from reportlab.lib.pagesizes import A5
from reportlab.pdfgen import canvas

out = sys.argv[1]
W, H = A5

def wrap(c, text, font, size, width):
    words, lines, cur = text.split(), [], ""
    for w in words:
        t = (cur + " " + w).strip()
        if c.stringWidth(t, font, size) > width:
            lines.append(cur); cur = w
        else:
            cur = t
    if cur: lines.append(cur)
    return lines

def story(path, header, title, meta, paras, chapters=None, outline=False):
    c = canvas.Canvas(path, pagesize=A5)
    c.setTitle(title)
    page = 1
    y = None
    def new_page():
        nonlocal y, page
        c.setFont("Helvetica", 8)
        c.drawString(40, H - 30, header.format(page))
        y = H - 60
    new_page()
    def line(t, font="Times-Roman", size=11, gap=15):
        nonlocal y, page
        if y < 50:
            c.showPage(); page += 1; new_page()
        c.setFont(font, size); c.drawString(40, y, t); y -= gap
    line(title, "Times-Bold", 20, 30)
    for m in meta: line(m, gap=18)
    for i, p in enumerate(paras):
        if chapters and i in chapters:
            name = chapters[i]
            if outline:
                key = f"k{i}"; c.bookmarkPage(key); c.addOutlineEntry(name, key, level=0)
            y -= 8; line(name, "Times-Bold", 15, 24)
        for l in wrap(c, p, "Times-Roman", 11, W - 80): line(l)
        y -= 6
    c.save()

paras = [
 "Im Jahr, in dem Mattea zwölf wird, dauert der Dezember gefühlt neun Wochen. Es liegt daran, dass ihr Vater seit August in einer anderen Stadt arbeitet und erst am 23. kommt.",
 "Die dreiundzwanzig Tage bis dahin bestehen aus Schule, Schnee, einem kaputten Rollladen und siebenundzwanzig Telefonaten.",
] + [f"Am {d}. Dezember waren es noch {23-d} Tage, und Mattea zählte weiter, obwohl Rückwärtszählen die Zeit nicht schneller macht, sondern langsamer, weil man jeden Tag daran erinnert wird, wie viele noch übrig sind. Sie hat trotzdem weitergezählt und jeden Abend einen Strich gemacht." for d in range(1, 23)]
story(f"{out}/kurzgeschichte-kopfzeile.pdf", "beispiel.example · kostenlose Geschichten zum Vorlesen · Seite {}",
      "Der längste Dezember", ["Vorlesedauer: etwa 7 Minuten", "Zum Vorlesen: ab 6 Jahren"], paras)
chap = {0: "Erstes Kapitel: Der Brief", 8: "Zweites Kapitel: Der Bahnhof", 16: "Drittes Kapitel: Heimkehr"}
story(f"{out}/kapitel-lesezeichen.pdf", "Der längste Dezember · Seite {}", "Der längste Dezember", ["Eine Geschichte"], paras, chap, outline=True)
story(f"{out}/kapitel-schriftgroesse.pdf", "Der längste Dezember · Seite {}", "Der längste Dezember", ["Eine Geschichte"], paras,
      {0: "Der Brief", 8: "Der Bahnhof", 16: "Heimkehr"}, outline=False)
