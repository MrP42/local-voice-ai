import Foundation
#if canImport(CryptoKit)
import CryptoKit

// Dateispeicher der Dokumentenablage auf dem iPhone (D5a-3, lva#441, Epic #395).
//
// Port der Dateiseite von `apps/local-voice/src-tauri/src/ablage/speicher_sync.rs` (`original_einlegen`,
// `text_einlegen`, `sync_bestand`, `sync_eingang_raeumen`) und `speicher.rs` (Ablagepfade). Gleiche Ordnernamen
// wie am Desktop unter `<wurzel>`:
//   ablage-zustand.json                Ledger + Bestand (AblageZustand), atomar geschrieben
//   dateien/<dat>/<fas>.<typ>          Originale (nur auf Anforderung)
//   text/<dat>/<textrev>.json          Textfassungen (Markenformat v1), Bytes unverändert
//   eingang/                           Zwischendateien laufender Downloads, nie im Baum
//
// Sicherheit: Kennungen, Endungen und `textrev` werden geprüft, bevor sie in einen Pfad gehen (nie ein `/` oder `..`).
// Nie durch eine Verknüpfung schreiben oder lesen (jeder Pfadteil ist ein echter Ordner, die Datei eine echte
// Datei). Nichts wird abgelegt, bevor Größe und SHA-256 stimmen; ein Fehler hinterlässt weder Teilergebnis noch
// Zwischendatei. Kein Logging, keine Namen oder Inhalte in Fehlern (Spec §10).
//
// Hinweis für D5b (eigene Importe): `bereinigen`, `entfernen` und `alleDateienEntfernen` löschen, was der Bestand nicht
// kennt. Noch nicht gesendete eigene Importe müssen dort ausgenommen werden, bevor es sie gibt.

public enum AblageSpeicherFehler: Error, Equatable, Sendable {
    /// Kennung, Endung oder `textrev` ist keine gültige Pfadkomponente.
    case ungueltigeKennung
    /// Schreiben nicht möglich (Datenträger voll, Schreibschutz, Verknüpfung auf dem Weg).
    case schreiben
    /// Größe, Hash oder Kopf der Datei passen nicht zum Eintrag.
    case passtNicht
}

extension AblageSpeicherFehler: LocalizedError {
    public var errorDescription: String? {
        switch self {
        case .ungueltigeKennung: return "Ablage: ungültige Kennung."
        case .schreiben: return "Ablage: Datei nicht speicherbar."
        case .passtNicht: return "Ablage: Datei passt nicht zu ihrem Eintrag."
        }
    }
}

/// Laufende Downloads (Zwischenpfade im Prozess): `eingangRaeumen` fasst sie nie an. Ohne das räumte ein Lauf den
/// `.part` eines gleichzeitig laufenden Original-Downloads weg. Gemeinsam für alle `AblageSpeicher`-Objekte (nach Pfad),
/// nie über ein `await` gehalten.
final class AblageZwischenRegister: @unchecked Sendable {
    static let shared = AblageZwischenRegister()
    private let lock = NSLock()
    private var aktiv = Set<String>()

    func anmelden(_ pfad: String) { lock.lock(); aktiv.insert(pfad); lock.unlock() }
    func abmelden(_ pfad: String) { lock.lock(); aktiv.remove(pfad); lock.unlock() }
    func istAktiv(_ pfad: String) -> Bool { lock.lock(); defer { lock.unlock() }; return aktiv.contains(pfad) }
}

/// Größe und SHA-256 (hex) einer heruntergeladenen Datei, vor der Sperre des Speichers berechnet.
public struct AblageOriginalPruefsumme: Equatable, Sendable {
    public let groesse: UInt64
    public let sha256Hex: String
}

public final class AblageSpeicher: @unchecked Sendable {
    public static let zustandsdatei = "ablage-zustand.json"
    static let dateienOrdner = "dateien"
    static let textOrdner = "text"
    static let eingangOrdner = "eingang"

    public let wurzel: URL
    private let fm = FileManager.default
    private let bestandSperre = NSLock()
    private let wartendeSperre = NSLock()
    private var wartende = 0

    public init(wurzel: URL) { self.wurzel = wurzel }

    /// Führt `body` aus, ohne dass ein anderer Aufrufer dieses Speichers gleichzeitig in einem `exklusiv`-Abschnitt ist. Der
    /// Lauf entfernt die Dateien begrabener Dateien und schreibt danach den Zustand (zwei Schritte, auf der Platte noch nicht
    /// zusammen sichtbar); ein gleichzeitiger Original-Download, der in dieser Lücke den Zustand liest, legte sonst ein
    /// Original einer begrabenen Datei ab, das niemand mehr wegräumt. Beide Seiten halten die Sperre über ihren ganzen
    /// Schritt. Nur kurze, synchrone Dateizugriffe darin, nie ein `await`; nicht wiedereintrittsfähig.
    func exklusiv<T>(_ body: () throws -> T) rethrows -> T {
        wartendeSperre.withLock { wartende += 1 }
        bestandSperre.lock()
        wartendeSperre.withLock { wartende -= 1 }
        defer { bestandSperre.unlock() }
        return try body()
    }

    /// Aufrufer von `exklusiv`, die gerade an der Sperre warten (damit Tests auf „wartet“ warten können, statt zu schlafen).
    var wartetAufExklusiv: Int { wartendeSperre.withLock { wartende } }

    // MARK: Dateisystem-Hilfen

    private func art(_ url: URL) -> FileAttributeType? { (try? fm.attributesOfItem(atPath: url.path))?[.type] as? FileAttributeType }
    private func istOrdner(_ url: URL) -> Bool { art(url) == .typeDirectory }
    private func istDatei(_ url: URL) -> Bool { art(url) == .typeRegular }

    private func unter(_ sub: String, _ dat: String) -> URL {
        wurzel.appendingPathComponent(sub, isDirectory: true).appendingPathComponent(dat, isDirectory: true)
    }

    /// `<wurzel>/<sub>/<dat>` als echter Ordner (angelegt, falls er fehlt); eine Verknüpfung auf dem Weg ist ein Fehler.
    private func ordnerSichern(_ sub: String, _ dat: String) throws -> URL {
        let root = wurzel.appendingPathComponent(sub, isDirectory: true)
        let dir = root.appendingPathComponent(dat, isDirectory: true)
        for u in [root, dir] {
            if let t = art(u) {
                guard t == .typeDirectory else { throw AblageSpeicherFehler.schreiben }
            } else {
                do { try fm.createDirectory(at: u, withIntermediateDirectories: true) } catch { throw AblageSpeicherFehler.schreiben }
                guard art(u) == .typeDirectory else { throw AblageSpeicherFehler.schreiben }
            }
        }
        return dir
    }

    /// Dateiname des Originals (`original_name`): `<fas>.<typ>`, ohne Endung nur `<fas>`. `nil` bei ungültiger Kennung.
    static func originalName(_ fas: String, _ typ: String) -> String? {
        guard AblageId.idGueltig(fas, prefix: AblageId.praefixFassung), AblageValidierung.typOk(typ) else { return nil }
        return typ.isEmpty ? fas : "\(fas).\(typ)"
    }

    public func originalURL(dat: String, fas: String, typ: String) -> URL? {
        guard AblageId.idGueltig(dat, prefix: AblageId.praefixDatei), let name = Self.originalName(fas, typ) else { return nil }
        return unter(Self.dateienOrdner, dat).appendingPathComponent(name)
    }

    public func textURL(dat: String, textrev: String) -> URL? {
        guard AblageId.idGueltig(dat, prefix: AblageId.praefixDatei), AblageValidierung.hex64(textrev) else { return nil }
        return unter(Self.textOrdner, dat).appendingPathComponent("\(textrev).json")
    }

    // MARK: Zustand

    private var zustandURL: URL { wurzel.appendingPathComponent(Self.zustandsdatei) }

    /// Lädt den Zustand für `konto`. Fehlt er oder ist er unlesbar, neuer oder von einem anderen Konto, ist er leer
    /// (nie ein Absturz). Eine unlesbare oder neuere Datei wird als `.defekt` beiseitegelegt, nie gelöscht; die eines
    /// anderen Kontos nicht (deren Daten sollen nicht liegen bleiben, sie werden beim nächsten Speichern ersetzt).
    public func laden(konto: String) -> (zustand: AblageZustand, verlust: AblageZustand.Verlust) {
        let url = zustandURL
        var bytes: [UInt8]?
        if art(url) != nil { bytes = (try? Data(contentsOf: url)).map { [UInt8]($0) } ?? [] }
        let r = AblageZustand.laden(bytes: bytes, konto: konto)
        if r.verlust == .unlesbar || r.verlust == .neuereVersion {
            try? fm.removeItem(at: defektURL)
            try? fm.moveItem(at: url, to: defektURL)
        }
        return r
    }

    private var defektURL: URL { wurzel.appendingPathComponent(Self.zustandsdatei + ".defekt") }

    /// Entfernt die beiseitegelegte unlesbare Zustandsdatei (sie kann Namen enthalten). Der Lauf tut das, sobald ein
    /// vollständiger Pull den Zustand neu aufgebaut hat oder das Konto gewechselt hat; bis dahin dient sie nur der Diagnose.
    public func defektEntfernen() { try? fm.removeItem(at: defektURL) }

    /// Schreibt den Zustand atomar (temporäre Datei, dann Umbenennen): bei Abbruch oder Fehler bleibt der alte Stand ganz.
    public func speichern(_ z: AblageZustand) throws {
        do {
            try fm.createDirectory(at: wurzel, withIntermediateDirectories: true)
            try Data(z.serialisieren()).write(to: zustandURL, options: .atomic)
        } catch {
            throw AblageSpeicherFehler.schreiben
        }
    }

    // MARK: Vorhanden

    /// Welche Originale und Texte dieses Gerät hat: nur echte Dateien in echten Ordnern, nie über eine Verknüpfung.
    public func vorhanden(_ b: AblageBestand) -> AblageVorhanden {
        var v = AblageVorhanden()
        let dateienOK = istOrdner(wurzel.appendingPathComponent(Self.dateienOrdner, isDirectory: true))
        let textOK = istOrdner(wurzel.appendingPathComponent(Self.textOrdner, isDirectory: true))
        for d in b.dateien.values where AblageId.idGueltig(d.id, prefix: AblageId.praefixDatei) {
            let odir = unter(Self.dateienOrdner, d.id), tdir = unter(Self.textOrdner, d.id)
            let oOK = dateienOK && istOrdner(odir), tOK = textOK && istOrdner(tdir)
            for f in d.fassungen {
                if oOK, let n = Self.originalName(f.fas, f.typ), istDatei(odir.appendingPathComponent(n)) { v.setzeOriginal(dat: d.id, fas: f.fas) }
                guard tOK else { continue }
                for t in f.texte where !t.blob.isEmpty && AblageValidierung.hex64(t.textrev) {
                    if istDatei(tdir.appendingPathComponent("\(t.textrev).json")) { v.setzeText(dat: d.id, textrev: t.textrev) }
                }
            }
        }
        return v
    }

    /// Pfad des Originals, wenn es hier als echte Datei in echten Ordnern liegt, sonst `nil`.
    public func originalLiegt(dat: String, fas: String, typ: String) -> URL? {
        guard let url = originalURL(dat: dat, fas: fas, typ: typ),
              istOrdner(wurzel.appendingPathComponent(Self.dateienOrdner, isDirectory: true)),
              istOrdner(url.deletingLastPathComponent()), istDatei(url) else { return nil }
        return url
    }

    /// Bytes einer geladenen Textfassung (höchstens 32 MiB), `nil` wenn sie hier nicht (als echte Datei) liegt.
    public func textLesen(dat: String, textrev: String) -> [UInt8]? {
        guard let url = textURL(dat: dat, textrev: textrev), istOrdner(wurzel.appendingPathComponent(Self.textOrdner, isDirectory: true)),
              istOrdner(url.deletingLastPathComponent()), istDatei(url),
              let size = (try? fm.attributesOfItem(atPath: url.path)[.size] as? NSNumber)?.intValue, size <= AblageTextfassung.maxBytes,
              let data = try? Data(contentsOf: url) else { return nil }
        return [UInt8](data)
    }

    // MARK: Einlegen

    /// Pfad für einen Download (im Ordner `eingang/`, nie im Baum).
    public func zwischenpfad() -> URL {
        let dir = wurzel.appendingPathComponent(Self.eingangOrdner, isDirectory: true)
        try? fm.createDirectory(at: dir, withIntermediateDirectories: true)
        let ziel = dir.appendingPathComponent(".sync-\(UUID().uuidString)")
        AblageZwischenRegister.shared.anmelden(ziel.path)    // gilt als laufender Download, bis `zwischenEntfernen`
        return ziel
    }

    /// Entfernt eine Zwischendatei und ihren Teil `.part` (so legt der Transport unfertige Downloads an).
    public func zwischenEntfernen(_ ziel: URL) {
        try? fm.removeItem(at: ziel)
        try? fm.removeItem(at: ziel.appendingPathExtension("part"))
        AblageZwischenRegister.shared.abmelden(ziel.path)
    }

    /// Entfernt liegengebliebene Downloads früherer Läufe. Zwischendateien laufender Downloads dieses Prozesses (auch der
    /// `.part` eines gleichzeitig laufenden Original-Downloads) bleiben unberührt.
    public func eingangRaeumen() {
        let dir = wurzel.appendingPathComponent(Self.eingangOrdner, isDirectory: true)
        guard let namen = try? fm.contentsOfDirectory(atPath: dir.path) else { return }
        for n in namen {
            let u = dir.appendingPathComponent(n)
            let basis = n.hasSuffix(".part") ? dir.appendingPathComponent(String(n.dropLast(5))) : u
            if AblageZwischenRegister.shared.istAktiv(basis.path) { continue }
            try? fm.removeItem(at: u)
        }
    }

    /// Legt eine heruntergeladene Textfassung ab, wenn sie zu ihrem Eintrag passt (`text_einlegen`): SHA-256 der Bytes =
    /// `Text.blob`, `textrev` und `original_sha256` im Kopf = Eintrag bzw. Fassung, zusätzlich `textrev` aus den Feldern
    /// des Eintrags neu berechnet. `false`: kein passender Eintrag (mehr). Wirft bei Abweichung; dann liegt nichts da.
    @discardableResult
    public func textEinlegen(bestand: AblageBestand, dat: String, fas: String, textrev: String, bytes: [UInt8]) throws -> Bool {
        guard AblageId.idGueltig(dat, prefix: AblageId.praefixDatei), AblageId.idGueltig(fas, prefix: AblageId.praefixFassung),
              AblageValidierung.hex64(textrev) else { throw AblageSpeicherFehler.ungueltigeKennung }
        guard let d = bestand.dateien[dat], let f = d.fassungen.first(where: { AblageBytes.equal($0.fas, fas) }),
              let t = f.texte.first(where: { AblageBytes.equal($0.textrev, textrev) }) else { return false }
        guard bytes.count <= AblageTextfassung.maxBytes, let tf = try? AblageTextfassung(bytes: bytes),
              tf.passt(zu: t, fassung: f), AblageTextfassung.textrevStimmt(text: t, fassung: f) else { throw AblageSpeicherFehler.passtNicht }
        let dir = try ordnerSichern(Self.textOrdner, dat)
        do { try Data(bytes).write(to: dir.appendingPathComponent("\(textrev).json"), options: .atomic) } catch { throw AblageSpeicherFehler.schreiben }
        return true
    }

    /// Größe und SHA-256 einer Zwischendatei, streamend und ohne Sperre des Speichers (`originalEinlegen` übernimmt das
    /// Ergebnis, statt unter der Sperre noch einmal über die ganze Datei zu lesen). Wirft `passtNicht`, wenn die Datei nicht
    /// lesbar ist.
    public func pruefsumme(zwischen: URL) throws -> AblageOriginalPruefsumme {
        guard istDatei(zwischen), let size = (try? fm.attributesOfItem(atPath: zwischen.path)[.size] as? NSNumber)?.uint64Value,
              let sha = try? Self.sha256Hex(datei: zwischen) else { throw AblageSpeicherFehler.passtNicht }
        return AblageOriginalPruefsumme(groesse: size, sha256Hex: sha)
    }

    /// Legt ein heruntergeladenes Original ohne Ersetzen an seinen Platz (`original_einlegen`). Größe und SHA-256 (streamend)
    /// müssen zur Fassung passen. `false`: Datei oder Fassung gibt es nicht mehr. Die Zwischendatei ist danach in jedem
    /// Fall weg (verschoben oder entfernt). `vorgeprueft`: Größe und Hash, die der Aufrufer schon vor der Sperre berechnet hat
    /// (`pruefsumme`); ohne sie wird hier gelesen. Die Größe wird immer frisch von der Datei genommen.
    @discardableResult
    public func originalEinlegen(bestand: AblageBestand, dat: String, fas: String, zwischen: URL,
                                 vorgeprueft: AblageOriginalPruefsumme? = nil) throws -> Bool {
        var fertig = false
        defer {
            if !fertig { try? fm.removeItem(at: zwischen) }
            AblageZwischenRegister.shared.abmelden(zwischen.path)
        }
        guard AblageId.idGueltig(dat, prefix: AblageId.praefixDatei), AblageId.idGueltig(fas, prefix: AblageId.praefixFassung) else {
            throw AblageSpeicherFehler.ungueltigeKennung
        }
        guard let d = bestand.dateien[dat], let f = d.fassungen.first(where: { AblageBytes.equal($0.fas, fas) }) else { return false }
        guard let ziel = originalURL(dat: dat, fas: fas, typ: f.typ) else { throw AblageSpeicherFehler.ungueltigeKennung }
        guard istDatei(zwischen), let size = (try? fm.attributesOfItem(atPath: zwischen.path)[.size] as? NSNumber)?.uint64Value,
              size == f.size else { throw AblageSpeicherFehler.passtNicht }
        let sha = try vorgeprueft?.sha256Hex ?? Self.sha256Hex(datei: zwischen)
        guard sha == f.blob else { throw AblageSpeicherFehler.passtNicht }
        let dir = try ordnerSichern(Self.dateienOrdner, dat)
        if art(ziel) != nil { return true }          // liegt schon: nicht ersetzen (die Zwischendatei räumt `defer`)
        do { try fm.moveItem(at: zwischen, to: dir.appendingPathComponent(ziel.lastPathComponent)) } catch {
            if art(ziel) != nil { return true }      // inzwischen von anderer Seite abgelegt
            throw AblageSpeicherFehler.schreiben
        }
        fertig = true
        return true
    }

    /// SHA-256 (hex) einer Datei, streamend (nie die ganze Datei im Speicher).
    static func sha256Hex(datei url: URL) throws -> String {
        let h = try FileHandle(forReadingFrom: url)
        defer { try? h.close() }
        var hasher = SHA256()
        while let stueck = try h.read(upToCount: 1 << 20), !stueck.isEmpty { hasher.update(data: stueck) }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    // MARK: Aufräumen

    /// Entfernt Originale und Texte der Dateien mit diesen Kennungen (begraben). Ungültige Kennungen werden ignoriert.
    public func entfernen(dateien ids: [String]) {
        for id in ids where AblageId.idGueltig(id, prefix: AblageId.praefixDatei) {
            for sub in [Self.dateienOrdner, Self.textOrdner] { try? fm.removeItem(at: unter(sub, id)) }
        }
    }

    /// Entfernt alles unter `dateien/` und `text/`, was der Bestand nicht (mehr) braucht: Ordner begrabener oder unbekannter
    /// Dateien, Dateien entfernter Fassungen oder nicht mehr eingetragener Texte, Zwischenreste, fremde Dateinamen,
    /// Verknüpfungen (die Verknüpfung selbst, nie ihr Ziel). Liefert die Zahl der entfernten Einträge.
    @discardableResult
    public func bereinigen(bestand b: AblageBestand) -> Int {
        raeumen(Self.dateienOrdner, b) { d in AblageMenge(d.fassungen.compactMap { Self.originalName($0.fas, $0.typ) }) }
            + raeumen(Self.textOrdner, b) { d in
                AblageMenge(d.fassungen.flatMap(\.texte).filter { !$0.blob.isEmpty && AblageValidierung.hex64($0.textrev) }.map { "\($0.textrev).json" })
            }
    }

    private func raeumen(_ sub: String, _ b: AblageBestand, erwartet: (AblageDatei) -> AblageMenge) -> Int {
        let root = wurzel.appendingPathComponent(sub, isDirectory: true)
        guard let t = art(root) else { return 0 }
        guard t == .typeDirectory else { return (try? fm.removeItem(at: root)) != nil ? 1 : 0 }
        var n = 0
        func weg(_ u: URL) { if (try? fm.removeItem(at: u)) != nil { n += 1 } }
        for name in (try? fm.contentsOfDirectory(atPath: root.path)) ?? [] {
            let dir = root.appendingPathComponent(name)
            guard AblageId.idGueltig(name, prefix: AblageId.praefixDatei), let d = b.dateien[name], istOrdner(dir) else { weg(dir); continue }
            let soll = erwartet(d)
            for datei in (try? fm.contentsOfDirectory(atPath: dir.path)) ?? [] {
                let u = dir.appendingPathComponent(datei)
                if !soll.contains(datei) || !istDatei(u) { weg(u) }
            }
        }
        return n
    }

    /// Entfernt den ganzen Speicherordner dieses Kontos (Zustand, `.defekt`, Originale, Texte, Zwischendateien): Abmelden.
    /// Der Aufrufer sorgt dafür, dass kein Lauf mehr schreibt (siehe `AblageDienst.verlassen`).
    public func alleEntfernen() {
        try? fm.removeItem(at: wurzel)
    }

    /// Entfernt alle Originale, Texte und Zwischendateien (Konto gewechselt); der Zustand bleibt.
    public func alleDateienEntfernen() {
        for sub in [Self.dateienOrdner, Self.textOrdner, Self.eingangOrdner] {
            try? fm.removeItem(at: wurzel.appendingPathComponent(sub, isDirectory: true))
        }
    }
}
#endif
