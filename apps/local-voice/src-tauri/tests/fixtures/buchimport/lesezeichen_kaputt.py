"""Kaputte Lesezeichen für den Buchimport (lva#476, Review): Ring über /Next,
Latin-1-Titel ohne BOM, doppelte Titel. Ohne Abhängigkeiten: `python3 lesezeichen_kaputt.py`."""
import sys
def build(outline_titles, cyclic=False):
    objs=[]
    def add(b): objs.append(b); return len(objs)
    # placeholders
    cat=add(b""); pages=add(b""); font=add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>")
    page_ids=[]; 
    texts=[b"Einf\xfchrung", b"Hauptteil"]
    for i,t in enumerate(texts):
        stream=b"BT /F1 18 Tf 72 700 Td ("+t+b") Tj ET BT /F1 11 Tf 72 650 Td (Das ist der Text der Seite.) Tj ET"
        c=add(b"<< /Length %d >>\nstream\n"%len(stream)+stream+b"\nendstream")
        p=add(b"")
        page_ids.append((p,c))
    out=add(b"")
    items=[]
    for i,t in enumerate(outline_titles):
        items.append(add(b""))
    for i,(t,oid) in enumerate(zip(outline_titles,items)):
        nxt=items[i+1] if i+1<len(items) else (items[0] if cyclic else None)
        d=b"<< /Title ("+t+b") /Parent %d 0 R /Dest [%d 0 R /XYZ 0 792 0]"%(out,page_ids[min(i,1)][0])
        if nxt: d+=b" /Next %d 0 R"%nxt
        d+=b" >>"
        objs[oid-1]=d
    objs[out-1]=b"<< /Type /Outlines /First %d 0 R /Last %d 0 R /Count %d >>"%(items[0],items[-1],len(items))
    for p,c in page_ids:
        objs[p-1]=b"<< /Type /Page /Parent %d 0 R /MediaBox [0 0 612 792] /Contents %d 0 R /Resources << /Font << /F1 %d 0 R >> >> >>"%(pages,c,font)
    objs[pages-1]=b"<< /Type /Pages /Kids [%s] /Count %d >>"%(b" ".join(b"%d 0 R"%p for p,_ in page_ids),len(page_ids))
    objs[cat-1]=b"<< /Type /Catalog /Pages %d 0 R /Outlines %d 0 R >>"%(pages,out)
    data=b"%PDF-1.4\n"; offs=[]
    for i,o in enumerate(objs):
        offs.append(len(data)); data+=b"%d 0 obj\n"%(i+1)+o+b"\nendobj\n"
    x=len(data)
    data+=b"xref\n0 %d\n0000000000 65535 f \n"%(len(objs)+1)+b"".join(b"%010d 00000 n \n"%o for o in offs)
    data+=b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n"%(len(objs)+1,x)
    return data
open("lesezeichen-latin1.pdf","wb").write(build([b"Einf\xfchrung", b"Hauptteil"]))
open("lesezeichen-ring.pdf","wb").write(build([b"Einf\xfchrung", b"Hauptteil"], cyclic=True))
open("lesezeichen-doppelt.pdf","wb").write(build([b"Anmerkungen", b"Anmerkungen", b"Hauptteil"]))
