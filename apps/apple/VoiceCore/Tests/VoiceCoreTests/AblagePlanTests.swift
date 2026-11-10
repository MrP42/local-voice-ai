import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Download-Plan, Budget, Anzeigezustand je Eintrag und Index-Tor der Ablage (D5a-3, lva#441, Abnahme 3 und 4).
// Reine Logik. Alles synthetisch.

private typealias W = AblageWelt
private let DAT = "ablage_datei"
private let ORD = "ablage_ordner"

/// Zustand mit einem lesbaren Ordner (`internal`) und den übergebenen Dateien.
private func zustand(_ dateien: [AblageDatei], ordner: [AblageOrdner]? = nil) -> AblageZustand {
    var z = W.neuerZustand()
    let o = ordner ?? [W.ordner(1, name: "Ablage", klasse: "internal")]
    W.einspielen(&z, ordner: o.map { W.remote(ordner: $0) }, dateien: dateien.map { W.remote(datei: $0) })
    return z
}

/// Datei mit einer Fassung und einem Text `ok` (Inhalt `abschnitte` bestimmt die Textgröße).
private func mitText(_ n: Int, ordner: String = W.ordId(1), typ: String = "pdf", extra: String = "") -> (datei: AblageDatei, text: AblageText, bytes: [UInt8]) {
    let f = W.fassung(n, nil, inhalt: "Original \(n)", typ: typ)
    let t = W.text(zu: f, abschnitte: [("zeile=1", "Text \(n) \(extra)")])
    var f2 = f
    f2.texte = [t.eintrag]
    return (W.datei(n, ordner: ordner, name: "d\(n).\(typ)", fassungen: [f2]), t.eintrag, t.bytes)
}

final class AblagePlanTests: XCTestCase {
    private let key = W.key

    // MARK: Plan

    func testNurTexteStandOkMitBlobUndHubKennungDesInhaltsHash() throws {
        let f = W.fassung(1, nil, inhalt: "A")
        let ok = W.text(zu: f, abschnitte: [("zeile=1", "ok")]).eintrag
        var keinText = W.text(zu: f, extraktor: "lva-text", version: "2", stand: "kein_text").eintrag
        keinText.blob = ""
        var fehler = W.text(zu: f, extraktor: "lva-text", version: "3", stand: "fehler").eintrag
        fehler.blob = ""
        fehler.code = "kaputt"
        var f2 = f
        f2.texte = [ok, keinText, fehler]
        let z = zustand([W.datei(1, ordner: W.ordId(1), fassungen: [f2])])
        let plan = AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: key)
        XCTAssertEqual(plan.count, 1)
        let a = try XCTUnwrap(plan.first)
        XCTAssertEqual(a.art, .text)
        XCTAssertEqual(a.dat, W.datId(1))
        XCTAssertEqual(a.fas, W.fasId(1))
        XCTAssertEqual(a.textrev, ok.textrev)
        XCTAssertEqual(a.sha256Hex, ok.blob)
        XCTAssertEqual(a.groesse, ok.size)
        XCTAssertEqual(a.hubId, AblageKrypto.hubBlobId(key: key, sha256Hex: ok.blob))
        XCTAssertEqual(a.hubId.count, 64)
    }

    func testOriginaleNurAufAnforderungUndNachAllenTexten() throws {
        let a = mitText(1), b = mitText(2, extra: String(repeating: "lang ", count: 50))
        let z = zustand([a.datei, b.datei])
        XCTAssertTrue(AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: key).allSatisfy { $0.art == .text })
        let angefordert = [AblageOriginalAnforderung(dat: a.datei.id, fas: W.fasId(1))]
        let plan = AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: key, angefordert: angefordert)
        XCTAssertEqual(plan.map(\.art), [.text, .text, .original])
        let orig = try XCTUnwrap(plan.last)
        XCTAssertEqual(orig.dat, a.datei.id)
        XCTAssertEqual(orig.fas, W.fasId(1))
        XCTAssertEqual(orig.groesse, a.datei.fassungen[0].size)
        XCTAssertEqual(orig.sha256Hex, a.datei.fassungen[0].blob)
        XCTAssertEqual(orig.hubId, AblageKrypto.hubBlobId(key: key, sha256Hex: a.datei.fassungen[0].blob))
        // Bereits vorhandenes Original wird nicht erneut angefordert.
        var da = AblageVorhanden()
        da.setzeOriginal(dat: a.datei.id, fas: W.fasId(1))
        XCTAssertEqual(AblageDownloadPlan.auftraege(zustand: z, vorhanden: da, key: key, angefordert: angefordert).map(\.art), [.text, .text])
    }

    func testKleinereZuerstDannByteOrdnung() {
        let gross = mitText(1, extra: String(repeating: "x", count: 500))
        let klein = mitText(2)
        let gleichA = mitText(4, extra: "xx"), gleichB = mitText(3, extra: "xx") // gleiche Größe, andere Kennung
        XCTAssertEqual(gleichA.text.size, gleichB.text.size)
        let z = zustand([gross.datei, klein.datei, gleichA.datei, gleichB.datei])
        let plan = AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: key)
        XCTAssertEqual(plan.map(\.groesse), plan.map(\.groesse).sorted())
        XCTAssertEqual(plan.last?.dat, W.datId(1), "größter zuletzt")
        let gleich = plan.filter { $0.groesse == gleichA.text.size }.map(\.dat)
        XCTAssertEqual(gleich, [W.datId(3), W.datId(4)], "bei gleicher Größe nach Kennung (Bytes)")
    }

    func testVorhandeneBegrabeneUnbekannteUndAngehalteneFallenWeg() throws {
        let vorhanden = mitText(1)
        let begraben = mitText(2)
        let angehalten = mitText(3)
        let halten = mitText(4, ordner: W.ordId(2))       // Ordner mit unbekannter Klasse
        let normal = mitText(5)
        let zuGross = mitText(6)
        var z = zustand([vorhanden.datei, begraben.datei, angehalten.datei, halten.datei, normal.datei, zuGross.datei],
                        ordner: [W.ordner(1, name: "A", klasse: "internal"), W.ordner(2, name: "B", klasse: "geheimnisvoll")])
        // Textgröße über der Lesegrenze (32 MiB) wird nie geladen.
        var f = z.bestand.dateien[zuGross.datei.id]!.fassungen[0]
        f.texte[0].size = UInt64(AblageTextfassung.maxBytes) + 1
        z.bestand.dateien[zuGross.datei.id]!.fassungen[0] = f
        W.einspielen(&z, dateien: [W.remoteGrab(collection: DAT, id: begraben.datei.id, revision: 2)])
        // Angehalten: unlesbarer neuerer Hub-Stand.
        var kaputt = W.remote(datei: angehalten.datei, revision: 2)
        kaputt.payload = "AAAA"
        W.einspielen(&z, dateien: [kaputt])
        var da = AblageVorhanden()
        da.setzeText(dat: vorhanden.datei.id, textrev: vorhanden.text.textrev)
        let plan = AblageDownloadPlan.auftraege(zustand: z, vorhanden: da, key: key)
        XCTAssertEqual(plan.map(\.dat), [normal.datei.id])
        // Ein Blob, der kein 64-stelliger Hex-Hash ist, hat keine Hub-Kennung: übersprungen.
        var z2 = zustand([normal.datei])
        z2.bestand.dateien[normal.datei.id]!.fassungen[0].texte[0].blob = "xyz"
        XCTAssertTrue(AblageDownloadPlan.auftraege(zustand: z2, vorhanden: AblageVorhanden(), key: key).isEmpty)
    }

    func testNichtAmHubLebendeDateienFallenWeg() {
        let a = mitText(1)
        var z = zustand([a.datei])
        z.ledger.objekte[AblageLedger.schluessel(DAT, a.datei.id)] = AblageObjStand(revision: 0)   // nie am Hub gesehen
        XCTAssertTrue(AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: key).isEmpty)
        z.ledger.objekte[AblageLedger.schluessel(DAT, a.datei.id)] = AblageObjStand(revision: 3, deleted: true)
        XCTAssertTrue(AblageDownloadPlan.auftraege(zustand: z, vorhanden: AblageVorhanden(), key: key).isEmpty)
    }

    func testOriginalAnforderungPruefungen() {
        let a = mitText(1)
        let z = zustand([a.datei])
        func auftrag(_ dat: String, _ fas: String) -> AblageAuftrag? {
            AblageDownloadPlan.original(zustand: z, dat: dat, fas: fas, key: key)
        }
        XCTAssertNotNil(auftrag(a.datei.id, W.fasId(1)))
        XCTAssertNil(auftrag(W.datId(9), W.fasId(1)), "unbekannte Datei")
        XCTAssertNil(auftrag(a.datei.id, W.fasId(9)), "unbekannte Fassung")
        XCTAssertNil(auftrag("dat_../x", W.fasId(1)), "ungültige Kennung")
        // Angehalten oder nicht am Hub: kein Auftrag.
        var z2 = z
        z2.ledger.objekte[AblageLedger.schluessel(DAT, a.datei.id)]?.angehalten = true
        XCTAssertNil(AblageDownloadPlan.original(zustand: z2, dat: a.datei.id, fas: W.fasId(1), key: key))
        // Zu viele Stücke (mehr als 4096 × 3 MiB): ungültiger Dateiverweis wie am Desktop.
        var z3 = z
        z3.bestand.dateien[a.datei.id]!.fassungen[0].size = UInt64(4097) * UInt64(LibrarySyncFormat.chunkPlainBytes)
        XCTAssertNil(AblageDownloadPlan.original(zustand: z3, dat: a.datei.id, fas: W.fasId(1), key: key))
    }

    func testBudgetErsteDateiImmerDanachNurWasPasst() {
        var b = AblageDownloadBudget(rest: 100)
        XCTAssertTrue(b.darfStarten(groesse: 500), "mindestens eine Datei je Lauf")
        b.verbraucht(groesse: 500)
        XCTAssertEqual(b.rest, 0, "kein Unterlauf")
        XCTAssertFalse(b.darfStarten(groesse: 1))
        var c = AblageDownloadBudget(rest: 100)
        XCTAssertTrue(c.darfStarten(groesse: 60)); c.verbraucht(groesse: 60)
        XCTAssertTrue(c.darfStarten(groesse: 40)); c.verbraucht(groesse: 40)
        XCTAssertFalse(c.darfStarten(groesse: 1))
        XCTAssertEqual(AblageDownloadBudget.standard, 256 * 1024 * 1024)
        // Ein fehlgeschlagener Download verbraucht nichts: die nächste große Datei ist weiter die „erste“.
        let d = AblageDownloadBudget(rest: 10)
        XCTAssertTrue(d.darfStarten(groesse: 50))
        XCTAssertTrue(d.darfStarten(groesse: 60))
    }

    // MARK: Textformate

    func testTextFormateNurDieFormateMitTextfassung() {
        for typ in ["pdf", "PDF", "docx", "docm", "odt", "pptx", "pptm", "odp", "xlsx", "xlsm", "ods", "txt", "md", "markdown", "csv", "tsv", "log", "html", "htm", "xhtml", "eml"] {
            XCTAssertTrue(AblageFormat.hatTextfassung(typ: typ), typ)
        }
        for typ in ["png", "jpg", "heic", "zip", "", "pdfx", "doc"] { XCTAssertFalse(AblageFormat.hatTextfassung(typ: typ), typ) }
    }

    // MARK: Wartet auf Text (vereinfachte Regel, keine erwartete textrev)

    func testWartetAufTextRegeln() {
        func datei(_ typ: String = "pdf", texte: [AblageText]) -> AblageDatei {
            var f = W.fassung(1, nil, inhalt: "A", typ: typ)
            f.texte = texte
            return W.datei(1, ordner: W.ordId(1), fassungen: [f])
        }
        let f = W.fassung(1, nil, inhalt: "A")
        let ok = W.text(zu: f).eintrag
        var okAlt = W.text(zu: f, version: "0").eintrag; okAlt.extraktorVersion = "0"
        var fehler = W.text(zu: f, version: "9", stand: "fehler").eintrag; fehler.blob = ""
        var kein = W.text(zu: f, version: "8", stand: "kein_text").eintrag; kein.blob = ""
        var da = AblageVorhanden()
        XCTAssertTrue(AblageAnzeige.wartetAufText(datei(texte: []), vorhanden: da), "eigener Import: Text wird am PC erstellt")
        XCTAssertFalse(AblageAnzeige.wartetAufText(datei("png", texte: []), vorhanden: da), "Format ohne Textfassung")
        XCTAssertTrue(AblageAnzeige.wartetAufText(datei(texte: [ok]), vorhanden: da), "Text eingetragen, noch nicht geladen")
        da.setzeText(dat: W.datId(1), textrev: ok.textrev)
        XCTAssertFalse(AblageAnzeige.wartetAufText(datei(texte: [ok]), vorhanden: da), "Text geladen")
        XCTAssertFalse(AblageAnzeige.wartetAufText(datei(texte: [kein]), vorhanden: AblageVorhanden()), "kein_text ist ein Ergebnis")
        XCTAssertFalse(AblageAnzeige.wartetAufText(datei(texte: [fehler]), vorhanden: AblageVorhanden()), "Fehler der Extraktion ist kein Warten")
        XCTAssertTrue(AblageAnzeige.wartetAufText(datei(texte: [fehler, ok]), vorhanden: AblageVorhanden()), "ein Fehler und ein Text, der noch kommt")
        // Eine ältere Textfassung (andere Extraktorversion) gilt, sobald sie hier liegt: die Tabelle der Version steht nirgends fest.
        var alt = AblageVorhanden()
        alt.setzeText(dat: W.datId(1), textrev: okAlt.textrev)
        XCTAssertFalse(AblageAnzeige.wartetAufText(datei(texte: [okAlt, ok]), vorhanden: alt))
        // Format-Endung wird ohne Rücksicht auf Groß/Klein erkannt.
        XCTAssertTrue(AblageAnzeige.wartetAufText(datei("PDF", texte: []), vorhanden: AblageVorhanden()))
    }

    func testWartetAufTextNimmtBeiKonfliktDieJuengsteFassung() {
        let a = W.fassung(1, nil, inhalt: "A", zeit: 1_000)
        var b = W.fassung(2, nil, inhalt: "B", zeit: 2_000)        // jüngster Kopf, mit Text
        let t = W.text(zu: b).eintrag
        b.texte = [t]
        let d = W.datei(1, ordner: W.ordId(1), fassungen: [a, b])   // zwei Köpfe: Konflikt
        XCTAssertTrue(AblageZusammenfuehren.konflikt(d))
        var da = AblageVorhanden()
        da.setzeText(dat: d.id, textrev: t.textrev)
        XCTAssertFalse(AblageAnzeige.wartetAufText(d, vorhanden: da), "zählt die jüngste Fassung, die Text hat")
        XCTAssertTrue(AblageAnzeige.wartetAufText(d, vorhanden: AblageVorhanden()))
    }

    // MARK: Zustand je Eintrag

    func testZustandJeEintragAbnahme3() throws {
        let sync = mitText(1)                                     // synchronisiert (Text geladen)
        let konflikt = W.datei(2, ordner: W.ordId(1), name: "k.pdf", fassungen: [W.fassung(1, nil, inhalt: "A"), W.fassung(2, nil, inhalt: "B", zeit: 2_000)])
        let wartet = W.datei(3, ordner: W.ordId(1), name: "w.pdf", fassungen: [W.fassung(3, nil, inhalt: "C")])   // texte = []
        let lokal = W.datei(4, ordner: W.ordId(1), name: "l.pdf", fassungen: [W.fassung(4, nil, inhalt: "D")], nurGeraetMin: true)
        let vertraulich = W.datei(5, ordner: W.ordId(1), name: "v.pdf", fassungen: [W.fassung(5, nil, inhalt: "E")], klasse: "confidential")
        let unbekannt = W.datei(6, ordner: W.ordId(1), name: "u.pdf", fassungen: [W.fassung(6, nil, inhalt: "F")], klasse: "geheimnisvoll")
        let bild = W.datei(7, ordner: W.ordId(1), name: "b.png", fassungen: [W.fassung(7, nil, inhalt: "G", typ: "png")])
        var z = zustand([sync.datei, konflikt, wartet, lokal, vertraulich, unbekannt, bild])
        var da = AblageVorhanden()
        da.setzeText(dat: sync.datei.id, textrev: sync.text.textrev)
        let s = AblageAnzeige.zustaende(z, vorhanden: da)
        XCTAssertEqual(s.dateien[sync.datei.id], .synchronisiert)
        XCTAssertEqual(s.dateien[konflikt.id], .konflikt)
        XCTAssertEqual(s.dateien[wartet.id], .wartetAufText)
        XCTAssertEqual(s.dateien[lokal.id], .nurGeraet)
        XCTAssertEqual(s.dateien[vertraulich.id], .nurGeraet)
        XCTAssertEqual(s.dateien[unbekannt.id], .angehalten, "unbekannte Klasse: Halten")
        XCTAssertEqual(s.dateien[bild.id], .synchronisiert, "Format ohne Textfassung wartet nicht")
        XCTAssertEqual(s.ordner[W.ordId(1)], .synchronisiert)
        // Konflikt geht vor „wartet auf Text“, angehalten geht vor allem.
        z.ledger.objekte[AblageLedger.schluessel(DAT, konflikt.id)]?.angehalten = true
        XCTAssertEqual(AblageAnzeige.zustaende(z, vorhanden: da).dateien[konflikt.id], .angehalten)
    }

    func testOrdnerZustaende() {
        var z = zustand([], ordner: [W.ordner(1, name: "A", klasse: "internal"), W.ordner(2, name: "B", klasse: "personal"),
                                     W.ordner(3, name: "C", nurGeraet: true), W.ordner(4, name: "D", klasse: "unklar")])
        z.ledger.objekte[AblageLedger.schluessel(ORD, W.ordId(1))]?.angehalten = true
        let s = AblageAnzeige.zustaende(z, vorhanden: AblageVorhanden()).ordner
        XCTAssertEqual(s[W.ordId(1)], .angehalten, "angehaltener Hub-Stand geht vor")
        XCTAssertEqual(s[W.ordId(2)], .nurGeraet)
        XCTAssertEqual(s[W.ordId(3)], .nurGeraet)
        XCTAssertEqual(s[W.ordId(4)], .angehalten)
    }

    func testUebersichtZeigtNamenKlassePapierkorbKonflikt() throws {
        let kind = W.ordner(2, name: "Verträge", eltern: W.ordId(1), klasse: "erbt")
        let ordner = [W.ordner(1, name: "Ablage", klasse: "internal"), kind]
        let konflikt = W.datei(1, ordner: kind.id, name: "Mietvertrag.pdf", fassungen: [W.fassung(1, nil, inhalt: "A", zeit: 1_000), W.fassung(2, nil, inhalt: "B", zeit: 2_000)], klasse: "public")
        let korb = W.datei(2, ordner: kind.id, name: "alt.docx", fassungen: [W.fassung(3, nil, inhalt: "C", typ: "docx")], korb: 1_700_000_000_000)
        let z = zustand([konflikt, korb], ordner: ordner)
        let u = AblageAnzeige.uebersicht(z, vorhanden: AblageVorhanden())
        let k = try XCTUnwrap(u.dateien.first { $0.id == konflikt.id })
        XCTAssertEqual(k.name, "Mietvertrag.pdf")
        XCTAssertEqual(k.klasse, .internal, "Klasse der Kette (die Mindestklasse public wird angehoben)")
        XCTAssertEqual(k.zustand, .konflikt)
        XCTAssertEqual(k.koepfe, [W.fasId(1), W.fasId(2)])
        XCTAssertFalse(k.imKorb)
        XCTAssertEqual(k.ordner, kind.id)
        let a = try XCTUnwrap(u.dateien.first { $0.id == korb.id })
        XCTAssertTrue(a.imKorb)
        XCTAssertEqual(a.typ, "docx")
        let o = try XCTUnwrap(u.ordner.first { $0.id == kind.id })
        XCTAssertEqual(o.name, "Verträge")
        XCTAssertEqual(o.klasse, .internal)
        XCTAssertEqual(o.eltern, .ordner(W.ordId(1)))
        XCTAssertEqual(u.angehalten, [])
    }

    // MARK: Index-Tor (Abnahme 4: kein Index-Eintrag für Unlesbares)

    func testIndexTorLaesstNurLebendeSynchroneLesbareDateienMitTextDurch() throws {
        let gut = mitText(1)
        let korb = mitText(2)
        let lokal = mitText(3)
        let kaputt = mitText(4)
        let ohneText = W.datei(5, ordner: W.ordId(1), fassungen: [W.fassung(5, nil, inhalt: "E")])
        let gefaelscht = mitText(6)
        var z = zustand([gut.datei, korb.datei, lokal.datei, kaputt.datei, ohneText, gefaelscht.datei])
        z.bestand.dateien[korb.datei.id]!.papierkorb = AblageRegister(wert: 1_700_000_000_000, zaehler: 2, geraet: W.geraet)
        z.bestand.dateien[lokal.datei.id]!.nurGeraetMin = true
        // kaputt: ein neuerer, unlesbarer Hub-Stand; gefälscht: Grabstein ohne Siegel.
        var k = W.remote(datei: kaputt.datei, revision: 2); k.payload = "AAAA"
        W.einspielen(&z, dateien: [k, W.remoteGrab(collection: DAT, id: gefaelscht.datei.id, revision: 2, echt: false)])
        var da = AblageVorhanden()
        for t in [gut, korb, lokal, kaputt, gefaelscht] { da.setzeText(dat: t.datei.id, textrev: t.text.textrev) }
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [gut.datei.id])
        XCTAssertTrue(AblageAnzeige.darfIndiziert(z, dat: gut.datei.id))
        for n in [korb, lokal, kaputt, gefaelscht] { XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: n.datei.id), n.datei.id) }
        XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: W.datId(99)), "unbekannt")
        // Ohne geladenen Text kein Index-Eintrag, auch bei sonst erlaubter Datei.
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: AblageVorhanden()), [])
        // Begraben (Grabstein mit Siegel): weder im Bestand noch indizierbar.
        W.einspielen(&z, dateien: [W.remoteGrab(collection: DAT, id: gut.datei.id, revision: 2)])
        XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: gut.datei.id))
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [])
    }

    func testIndexTorSchliesstHaltenAusUndWartetKeinTextVonAnderenAus() {
        let halten = mitText(1, ordner: W.ordId(2))
        let z = zustand([halten.datei], ordner: [W.ordner(1, klasse: "internal"), W.ordner(2, klasse: "unklar")])
        var da = AblageVorhanden()
        da.setzeText(dat: halten.datei.id, textrev: halten.text.textrev)
        XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: halten.datei.id))
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [])
    }

    func testIndexTorSperrtBaumUnterAngehaltenemOrdner() {
        // Der Hub-Stand eines Ordners ist unlesbar (der PC kann die Klasse angehoben haben): nichts darunter indizieren,
        // auch wenn der Ordner lokal fehlt oder noch in alter Fassung da ist.
        let a = mitText(1, ordner: W.ordId(2))        // Ordner 2 fehlt lokal (zählt wie im Papierkorb), Hub-Stand angehalten
        let b = mitText(2, ordner: W.ordId(3))        // Ordner 3 lokal vorhanden, Hub-Stand angehalten
        let c = mitText(3, ordner: W.ordId(4))        // unauffällig
        var z = zustand([a.datei, b.datei, c.datei], ordner: [W.ordner(1, klasse: "internal"), W.ordner(3, klasse: "internal"),
                                                              W.ordner(4, klasse: "internal")])
        var da = AblageVorhanden()
        for t in [a, b, c] { da.setzeText(dat: t.datei.id, textrev: t.text.textrev) }
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [b.datei.id, c.datei.id], "vorher offen (a: fehlender Ordner zählt wie Papierkorb)")
        z.ledger.objekte[AblageLedger.schluessel(ORD, W.ordId(2))] = AblageObjStand(revision: 3, angehalten: true)
        z.ledger.objekte[AblageLedger.schluessel(ORD, W.ordId(3))]?.angehalten = true
        XCTAssertEqual(AblageAnzeige.indexQuellen(z, vorhanden: da), [c.datei.id])
    }
}
#endif
