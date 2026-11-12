import SwiftUI
import QuickLook

// Seite „Ablage“ auf dem iPhone (D5a-4, lva#441, Epic #395), nach Spec §6 „iPhone (D5)“: Ordnerbaum und Liste mit Zuständen,
// Detail mit Fassungen und Konflikt-Hinweis, Vorschau des Originals nur auf Tippen (Regel „nur im WLAN“ aus D5a-3), Suche mit
// Sprung zum Anker, Kontingent im Fuß. Das iPhone liest nur (D5a): kein Anlegen, Umbenennen, Löschen, keine KI-Antworten.
//
// Alle Entscheidungen (Zustände, Baum, Treffer) fallen in VoiceCore (`AblageAnzeige`, `AblageBaum`, `AblageSuchdienst`); die Views
// zeichnen nur. Texte deutsch, Schrift folgt Dynamic Type (semantische Schriftgrößen, nichts fest), Zustände tragen Symbol UND Text
// (nie nur Farbe), jede Zeile hat ein VoiceOver-Label, Farben aus `VoicePalette` (hell und dunkel).

// MARK: - Bausteine

enum AblageAnzeigeText {
    static func klasse(_ k: AblageKlasse) -> String {
        switch k {
        case .public: return "Öffentlich"
        case .internal: return "Intern"
        case .confidential: return "Vertraulich"
        case .personal: return "Persönlich"
        case .unbekannt: return "Klasse unbekannt"
        }
    }

    static func zustand(_ z: AblageSyncZustand, imKorb: Bool = false) -> (symbol: String, text: String) {
        if imKorb { return ("trash", "Im Papierkorb") }
        switch z {
        case .synchronisiert: return ("checkmark.circle", "Synchronisiert")
        case .konflikt: return ("exclamationmark.triangle.fill", "Konflikt")
        case .wartetAufText: return ("clock", "Text wird am PC erstellt")
        case .nurGeraet: return ("lock", "Nur Gerät")
        case .angehalten: return ("pause.circle", "Angehalten")
        }
    }

    /// Ordnerpfad zum Vorlesen: „Verträge › Miete“ wird „Verträge, Miete“ (das Zeichen › liest VoiceOver nicht sinnvoll).
    static func pfadGesprochen(_ pfad: String) -> String {
        pfad.replacingOccurrences(of: " › ", with: ", ")
    }

    static func groesse(_ bytes: UInt64?) -> String? {
        bytes.map { ByteCountFormatter.string(fromByteCount: Int64(clamping: $0), countStyle: .file) }
    }

    static func typ(_ endung: String?) -> String? {
        guard let e = endung, !e.isEmpty else { return nil }
        return e.uppercased()
    }

    static func datum(_ ms: Int64) -> String {
        Date(timeIntervalSince1970: Double(ms) / 1000).formatted(date: .abbreviated, time: .shortened)
    }

    /// Fundstelle mit den Suchwörtern hervorgehoben (Groß/Klein und Akzente egal, wie die Suche selbst).
    static func hervorgehoben(_ auszug: String, woerter: [String]) -> AttributedString {
        var s = AttributedString(auszug)
        for w in Set(woerter) where !w.isEmpty {
            var von = auszug.startIndex
            while let r = auszug.range(of: w, options: [.caseInsensitive, .diacriticInsensitive], range: von..<auszug.endIndex) {
                if let a = AttributedString.Index(r.lowerBound, within: s), let b = AttributedString.Index(r.upperBound, within: s) {
                    s[a..<b].font = .body.weight(.semibold)
                    s[a..<b].backgroundColor = VoicePalette.brand.opacity(0.45)
                }
                von = r.upperBound
            }
        }
        return s
    }
}

/// Zustand als Symbol mit Text; die Farbe ergänzt nur.
struct AblageZustandsmarke: View {
    let zustand: AblageSyncZustand
    var imKorb = false

    var body: some View {
        let z = AblageAnzeigeText.zustand(zustand, imKorb: imKorb)
        Label(z.text, systemImage: z.symbol)
            .font(.caption)
            .foregroundStyle(zustand == .konflikt && !imKorb ? VoicePalette.danger : (zustand == .synchronisiert && !imKorb ? VoicePalette.success : VoicePalette.secondaryText))
            .labelStyle(.titleAndIcon)
    }
}

/// Eine Zeile der Liste: Dateiname, Angaben, Zustand.
struct AblageDateiZeile: View {
    let datei: AblageDateiEintrag
    var pfad: String?
    /// Das angeforderte Original ist an dieser Datei gescheitert (fehlt am Hub, passt nicht): „erneut antippen“.
    var nichtGeladen = false

    private var angaben: String {
        [AblageAnzeigeText.typ(datei.typ), AblageAnzeigeText.groesse(datei.groesse)].compactMap { $0 }.joined(separator: " · ")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "doc.text").foregroundStyle(VoicePalette.secondaryText).accessibilityHidden(true)
                Text(datei.name).font(.body).foregroundStyle(VoicePalette.text).fixedSize(horizontal: false, vertical: true)
            }
            if let pfad, !pfad.isEmpty { Text(pfad).font(.caption).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true) }
            HStack(spacing: 10) {
                if !angaben.isEmpty { Text(angaben).font(.caption).foregroundStyle(VoicePalette.secondaryText) }
                AblageZustandsmarke(zustand: datei.zustand, imKorb: datei.imKorb)
            }
            .fixedSize(horizontal: false, vertical: true)
            if nichtGeladen {
                Label("Original nicht geladen – erneut antippen", systemImage: "exclamationmark.arrow.circlepath")
                    .font(.caption).foregroundStyle(VoicePalette.danger).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(([datei.name, pfad.flatMap { $0.isEmpty ? nil : "in Ordner \(AblageAnzeigeText.pfadGesprochen($0))" },
                              AblageAnzeigeText.zustand(datei.zustand, imKorb: datei.imKorb).text, AblageAnzeigeText.typ(datei.typ), AblageAnzeigeText.groesse(datei.groesse),
                              nichtGeladen ? "Original nicht geladen, erneut antippen" : nil].compactMap { $0 }).joined(separator: ", "))
    }
}

// MARK: - Seite

struct AblageTab: View {
    var body: some View { NavigationStack { AblageStart() } }
}

private struct SuchSchluessel: Equatable {
    var frage: String
    var version: Int
}

struct AblageStart: View {
    @ObservedObject var ablage = AblageSync.shared
    @ObservedObject var konto = LibrarySync.shared
    @State private var frage = ""
    @State private var ergebnisse: [AblageSuchErgebnis]?
    @State private var eingeklappt: Set<String> = []
    @State private var korbOffen = false
    @State private var showSync = false
    /// Zeilen und Papierkorb des Baums, nur bei neuer Übersicht oder neuem Klappstand berechnet (nicht bei jeder Auswertung des Körpers).
    @State private var baumStand = AblageBaumZwischenspeicher()
    @ScaledMetric(relativeTo: .body) private var einzug: CGFloat = 16
    @Environment(\.dynamicTypeSize) private var typeSize

    private func gescheitert(_ d: AblageDateiEintrag) -> Bool {
        d.aktuelleFassung.map { ablage.fehlgeschlageneOriginale.contains(AblageSync.schluessel(dat: d.id, fas: $0)) } ?? false
    }

    private var suchbegriff: String { frage.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        List {
            if konto.account == nil {
                nichtVerbunden
            } else if suchbegriff.isEmpty {
                baum
            } else {
                suche
            }
            fuss
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .background(VoicePalette.background)
        .navigationTitle("Ablage")
        .navigationBarTitleDisplayMode(.large)
        .searchable(text: $frage, placement: .navigationBarDrawer(displayMode: .always), prompt: "Dateien und Inhalte suchen")
        .autocorrectionDisabled()
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button { showSync = true } label: {
                    Image(systemName: konto.account == nil ? "icloud.slash" : (konto.error != nil ? "exclamationmark.icloud" : "checkmark.icloud"))
                }.accessibilityLabel("Konto und Sync: \(konto.status)")
            }
            ToolbarItem(placement: .topBarTrailing) { SettingsButton() }
        }
        .sheet(isPresented: $showSync) { LibrarySyncSheet() }
        .refreshable { await ablage.run() }
        .task { await ablage.run() }
        .task(id: SuchSchluessel(frage: suchbegriff, version: ablage.indexVersion)) {
            guard !suchbegriff.isEmpty else { ergebnisse = nil; return }
            try? await Task.sleep(nanoseconds: 200_000_000)
            if Task.isCancelled { return }
            ergebnisse = await ablage.suchen(suchbegriff)
        }
    }

    // MARK: Baum und Liste

    @ViewBuilder private var baum: some View {
        if let u = ablage.uebersicht {
            let ansicht = baumStand.ansicht(u, version: ablage.indexVersion, eingeklappt: eingeklappt)
            let zeilen = ansicht.zeilen, korb = ansicht.korb
            if zeilen.isEmpty && korb.isEmpty {
                leer
            }
            ForEach(zeilen) { z in
                switch z {
                case .ordner(let id, let name, let tiefe, let klasse, let zustand, let anzahl, let offen):
                    ordnerZeile(id: id, name: name, tiefe: tiefe, klasse: klasse, zustand: zustand, anzahl: anzahl, offen: offen)
                case .datei(let d, let tiefe):
                    NavigationLink { AblageDateiDetail(dat: d.id) } label: { AblageDateiZeile(datei: d, nichtGeladen: gescheitert(d)) }
                        .accessibilityIdentifier("ablageDatei")
                        .listRowInsets(EdgeInsets(top: 2, leading: 16 + min(CGFloat(tiefe), typeSize.isAccessibilitySize ? 1 : 4) * einzug, bottom: 2, trailing: 16))
                }
            }
            if !korb.isEmpty { papierkorb(korb) }
        } else if ablage.running {
            HStack(spacing: 10) { ProgressView(); Text("Ablage wird abgeglichen …") }.accessibilityIdentifier("ablageLaedt")
        }
    }

    private func ordnerZeile(id: String, name: String, tiefe: Int, klasse: AblageKlasse, zustand: AblageSyncZustand, anzahl: Int, offen: Bool) -> some View {
        Button {
            if offen { eingeklappt.insert(id) } else { eingeklappt.remove(id) }
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Image(systemName: "chevron.right").font(.caption.weight(.semibold)).rotationEffect(.degrees(offen ? 90 : 0))
                    .foregroundStyle(VoicePalette.secondaryText).accessibilityHidden(true)
                Image(systemName: offen ? "folder.fill" : "folder").foregroundStyle(VoicePalette.accent).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(name).font(.headline).foregroundStyle(VoicePalette.text).fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 8) {
                        Text("\(anzahl) \(anzahl == 1 ? "Datei" : "Dateien")").font(.caption).foregroundStyle(VoicePalette.secondaryText)
                        if klasse != .unbekannt || id != AblageBaum.ohneOrdner { Text(AblageAnzeigeText.klasse(klasse)).font(.caption).foregroundStyle(VoicePalette.secondaryText) }
                        if zustand == .nurGeraet || zustand == .angehalten { AblageZustandsmarke(zustand: zustand) }
                    }.fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 0)
            }
            .contentShape(Rectangle())
            .frame(minHeight: 44)
        }
        .buttonStyle(.plain)
        .listRowInsets(EdgeInsets(top: 2, leading: 16 + min(CGFloat(tiefe), typeSize.isAccessibilitySize ? 1 : 4) * einzug, bottom: 2, trailing: 16))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(ordnerBeschriftung(id: id, name: name, klasse: klasse, zustand: zustand, anzahl: anzahl, offen: offen))
        .accessibilityHint(offen ? "Doppeltippen zum Schließen" : "Doppeltippen zum Öffnen")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("ablageOrdner")
    }

    /// VoiceOver-Text der Ordnerzeile: dasselbe, was sie zeigt, samt Zustand („Nur Gerät“, „Angehalten“); die Klasse fehlt wie auf dem
    /// Bildschirm bei „Ohne Ordner“.
    private func ordnerBeschriftung(id: String, name: String, klasse: AblageKlasse, zustand: AblageSyncZustand, anzahl: Int, offen: Bool) -> String {
        var teile = [name, "Ordner", "\(anzahl) \(anzahl == 1 ? "Datei" : "Dateien")"]
        if klasse != .unbekannt || id != AblageBaum.ohneOrdner { teile.append(AblageAnzeigeText.klasse(klasse)) }
        if zustand == .nurGeraet || zustand == .angehalten { teile.append(AblageAnzeigeText.zustand(zustand).text) }
        teile.append(offen ? "geöffnet" : "geschlossen")
        return teile.joined(separator: ", ")
    }

    @ViewBuilder private func papierkorb(_ korb: [AblageDateiEintrag]) -> some View {
        Button { korbOffen.toggle() } label: {
            HStack(spacing: 8) {
                Image(systemName: "chevron.right").font(.caption.weight(.semibold)).rotationEffect(.degrees(korbOffen ? 90 : 0))
                    .foregroundStyle(VoicePalette.secondaryText).accessibilityHidden(true)
                Image(systemName: "trash").foregroundStyle(VoicePalette.secondaryText).accessibilityHidden(true)
                Text("Papierkorb").font(.headline).foregroundStyle(VoicePalette.text)
                Text("\(korb.count)").font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                Spacer(minLength: 0)
            }.frame(minHeight: 44).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Papierkorb, \(korb.count) \(korb.count == 1 ? "Datei" : "Dateien"), \(korbOffen ? "geöffnet" : "geschlossen")")
        .accessibilityHint(korbOffen ? "Doppeltippen zum Schließen" : "Doppeltippen zum Öffnen")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("ablagePapierkorb")
        if korbOffen {
            ForEach(korb, id: \.id) { d in
                NavigationLink { AblageDateiDetail(dat: d.id) } label: { AblageDateiZeile(datei: d) }
                    .listRowInsets(EdgeInsets(top: 2, leading: 16 + einzug, bottom: 2, trailing: 16))
            }
        }
    }

    private var leer: some View {
        ContentUnavailableView {
            Label("Die Ablage ist leer", systemImage: "folder")
        } description: {
            Text("Dateien, die du am PC in der Ablage ablegst, erscheinen hier, sobald der Text dazu erstellt ist.")
        }.listRowBackground(Color.clear)
    }

    private var nichtVerbunden: some View {
        ContentUnavailableView {
            Label("Nicht verbunden", systemImage: "icloud.slash")
        } description: {
            Text("Verbinde dieses iPhone mit deinem Konto, um die Ablage deines PCs zu sehen. Am PC unter Einstellungen › Konto & Geräte auf „Handy verbinden“ gehen.")
        } actions: {
            Button("Konto & Sync") { showSync = true }.buttonStyle(.borderedProminent)
        }.listRowBackground(Color.clear)
    }

    // MARK: Suche

    @ViewBuilder private var suche: some View {
        if let e = ergebnisse {
            if e.isEmpty {
                ContentUnavailableView.search(text: suchbegriff).listRowBackground(Color.clear)
                if let a = ablage.abdeckung, a.ohneText > 0 || a.nichtDurchsuchbar > 0 {
                    Text("Nicht durchsucht: \(a.ohneText > 0 ? "\(a.ohneText) \(a.ohneText == 1 ? "Datei" : "Dateien") ohne Text (Text wird am PC erstellt)" : "")\(a.ohneText > 0 && a.nichtDurchsuchbar > 0 ? ", " : "")\(a.nichtDurchsuchbar > 0 ? "\(a.nichtDurchsuchbar) nur auf dem PC oder angehalten" : "").")
                        .font(.footnote).foregroundStyle(VoicePalette.secondaryText).accessibilityIdentifier("ablageKeineTreffer")
                }
            } else {
                let woerter = AblageIndex.anfrageWoerter(suchbegriff)
                ForEach(e) { r in
                    NavigationLink { AblageDateiDetail(dat: r.dat) } label: {
                        AblageDateiZeile(datei: r.eintrag, pfad: r.ordnerPfad, nichtGeladen: gescheitert(r.eintrag))
                    }.accessibilityIdentifier("ablageTreffer")
                    ForEach(r.stellen, id: \.anker) { s in
                        NavigationLink { AblageTextAnsicht(dat: r.dat, name: r.eintrag.name, anker: s.anker) } label: {
                            VStack(alignment: .leading, spacing: 3) {
                                Text(AblageAnker.beschreibung(s.anker)).font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
                                if !s.pfad.isEmpty { Text(s.pfad.joined(separator: " › ")).font(.caption).foregroundStyle(VoicePalette.secondaryText).lineLimit(2) }
                                Text(AblageAnzeigeText.hervorgehoben(s.auszug, woerter: woerter)).font(.body).foregroundStyle(VoicePalette.text).lineLimit(4)
                            }
                            .accessibilityElement(children: .ignore)
                            .accessibilityLabel("Fundstelle in \(r.eintrag.name)\(r.ordnerPfad.isEmpty ? "" : ", Ordner \(AblageAnzeigeText.pfadGesprochen(r.ordnerPfad))"), \(AblageAnker.beschreibung(s.anker)): \(s.auszug)")
                        }
                        .accessibilityIdentifier("ablageFundstelle")
                        .listRowInsets(EdgeInsets(top: 2, leading: 16 + einzug, bottom: 2, trailing: 16))
                    }
                }
            }
        } else {
            HStack(spacing: 10) { ProgressView(); Text("Suche …") }
        }
    }

    // MARK: Fuß

    @ViewBuilder private var fuss: some View {
        if konto.account != nil {
            VStack(alignment: .leading, spacing: 6) {
                if let a = ablage.abdeckung {
                    Text(a.text).font(.footnote).foregroundStyle(VoicePalette.secondaryText).accessibilityIdentifier("ablageAbdeckung")
                }
                if let k = ablage.kontingent {
                    ProgressView(value: k.anteil).tint(k.voll ? VoicePalette.danger : VoicePalette.accent).accessibilityHidden(true)
                    Text(k.text).font(.footnote).foregroundStyle(k.voll ? VoicePalette.danger : VoicePalette.secondaryText).accessibilityIdentifier("ablageKontingent")
                }
                Text(ablage.status).font(.footnote).foregroundStyle(VoicePalette.secondaryText).accessibilityIdentifier("ablageStatus")
                if let e = ablage.error { Text(e).font(.footnote).foregroundStyle(VoicePalette.danger) }
            }
            .padding(.vertical, 6)
            .listRowSeparator(.hidden)
            .accessibilityElement(children: .contain)
        }
    }
}

// MARK: - Detail

struct AblageDateiDetail: View {
    let dat: String
    @ObservedObject var ablage = AblageSync.shared
    @ObservedObject var konto = LibrarySync.shared
    @State private var detail: AblageDetail?
    @State private var geladen = false
    @State private var vorschau: AblageVorschauZiel?
    @State private var laedt = false
    @State private var hinweis: String?

    var body: some View {
        List {
            if let d = detail {
                kopf(d)
                hinweise(d)
                aktionen(d)
                Section("Fassungen") {
                    ForEach(d.fassungen, id: \.fas) { f in fassungsZeile(f, konflikt: d.konflikt) }
                }
            } else if geladen {
                ContentUnavailableView("Datei nicht mehr vorhanden", systemImage: "questionmark.folder",
                                       description: Text("Sie wurde am PC gelöscht oder ist hier nicht mehr bekannt."))
            } else {
                ProgressView()
            }
        }
        .listStyle(.insetGrouped)
        .scrollContentBackground(.hidden)
        .background(VoicePalette.background)
        .navigationTitle("Datei")
        .navigationBarTitleDisplayMode(.inline)
        .fullScreenCover(item: $vorschau) { z in AblageVorschau(ziel: z) { vorschau = nil }.ignoresSafeArea() }
        .task(id: ablage.indexVersion) { detail = await ablage.detail(dat: dat); geladen = true }
    }

    private func kopf(_ d: AblageDetail) -> some View {
        Section {
            VStack(alignment: .leading, spacing: 8) {
                Text(d.name).font(.title3.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
                if !d.ordnerPfad.isEmpty {
                    Label(d.ordnerPfad.joined(separator: " › "), systemImage: "folder").font(.subheadline).foregroundStyle(VoicePalette.secondaryText)
                }
                HStack(spacing: 12) {
                    AblageZustandsmarke(zustand: d.zustand, imKorb: d.imKorb)
                    Text(AblageAnzeigeText.klasse(d.klasse)).font(.caption).foregroundStyle(VoicePalette.secondaryText)
                }.fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("ablageDetailKopf")
        }
    }

    @ViewBuilder private func hinweise(_ d: AblageDetail) -> some View {
        if d.imKorb {
            hinweiskarte("trash", "Diese Datei liegt im Papierkorb. Sie wird nicht durchsucht. Wiederherstellen und endgültig Löschen geht am PC.")
        }
        if d.konflikt && !d.imKorb {
            hinweiskarte("exclamationmark.triangle.fill", "Mehrere Fassungen: Diese Datei wurde auf verschiedenen Geräten gleichzeitig geändert. Welche gilt, entscheidest du am PC. Hier wird die jüngste Fassung angezeigt und durchsucht."
                + (d.konfliktOhneGemeinsame ? " Es gibt keine gemeinsame Vorgängerfassung; der PC nutzt die Datei erst nach deiner Wahl für KI." : ""), warnung: true)
        }
        if d.zustand == .wartetAufText && !d.imKorb {
            hinweiskarte("clock", "Text wird am PC erstellt. Bis dahin ist die Datei nicht durchsuchbar. Das iPhone erstellt nie selbst Text aus Dateien.")
        }
        if d.zustand == .angehalten && !d.imKorb {
            hinweiskarte("pause.circle", "Der Stand dieser Datei ist hier nicht lesbar (andere Schlüssel, neuere Version oder unbekannte Klasse). Es wird nichts geändert und nichts durchsucht.")
        }
        if d.zustand == .nurGeraet && !d.imKorb {
            hinweiskarte("lock", "Nur Gerät: Diese Datei verlässt den PC nicht und wird hier nicht durchsucht.")
        }
    }

    private func hinweiskarte(_ symbol: String, _ text: String, warnung: Bool = false) -> some View {
        Section {
            Label { Text(text).font(.subheadline).fixedSize(horizontal: false, vertical: true) } icon: {
                Image(systemName: symbol).foregroundStyle(warnung ? VoicePalette.danger : VoicePalette.secondaryText)
            }
            .accessibilityElement(children: .combine)
            .accessibilityIdentifier("ablageHinweis")
        }
    }

    @ViewBuilder private func aktionen(_ d: AblageDetail) -> some View {
        let aktuell = d.fassungen.first(where: \.istAktuell)
        if !d.imKorb, let a = aktuell {
            Section {
                if a.text == .bereit && d.zustand != .angehalten && d.zustand != .nurGeraet {
                    NavigationLink { AblageTextAnsicht(dat: dat, name: d.name, anker: nil) } label: { Label("Text ansehen", systemImage: "text.alignleft") }
                        .accessibilityIdentifier("ablageTextAnsehen")
                }
                if d.zustand != .angehalten && d.zustand != .nurGeraet {
                    let schluessel = AblageSync.schluessel(dat: dat, fas: a.fas)
                    let gescheitert = ablage.fehlgeschlageneOriginale.contains(schluessel)
                    Button {
                        Task { await originalOeffnen(a) }
                    } label: {
                        HStack {
                            Label(gescheitert ? "Original nicht geladen – erneut antippen" : (a.originalVorhanden ? "Original ansehen" : "Original laden und ansehen"),
                                  systemImage: gescheitert ? "exclamationmark.arrow.circlepath" : "eye")
                            Spacer()
                            if laedt { ProgressView() }
                        }
                    }
                    .disabled(laedt)
                    .accessibilityIdentifier("ablageOriginal")
                    .accessibilityHint(a.originalVorhanden ? "Öffnet die Vorschau" : "Lädt die Datei auf dieses iPhone, nur im WLAN")
                    if let h = hinweis {
                        Text(h).font(.footnote).foregroundStyle(VoicePalette.danger).accessibilityIdentifier("ablageOriginalHinweis")
                    } else if !a.originalVorhanden {
                        Text("Originale werden nur auf Tippen geladen, bei „nur im WLAN“ nicht über Mobilfunk.").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                    }
                }
            }
        }
    }

    private func originalOeffnen(_ f: AblageFassungSicht) async {
        laedt = true; hinweis = nil
        defer { laedt = false }
        do {
            let url = try await ablage.originalLaden(dat: dat, fas: f.fas)
            vorschau = AblageVorschauZiel(url: url, titel: detail?.name ?? "Original")
        } catch let e as AblageLaufFehler {
            hinweis = e.errorDescription
        } catch {
            hinweis = "Das Original konnte nicht geladen werden."
        }
    }

    private func fassungsZeile(_ f: AblageFassungSicht, konflikt: Bool) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(AblageAnzeigeText.datum(f.zeit)).font(.body)
                if f.istAktuell { Text("Aktuell").font(.caption.weight(.semibold)).padding(.horizontal, 6).padding(.vertical, 2)
                    .background(VoicePalette.border, in: Capsule()) }
                if konflikt && f.istKopf { Text("Offen").font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.danger) }
            }.fixedSize(horizontal: false, vertical: true)
            Text([AblageAnzeigeText.typ(f.typ), AblageAnzeigeText.groesse(f.groesse), textStand(f.text), f.originalVorhanden ? "Original geladen" : nil].compactMap { $0 }.joined(separator: " · "))
                .font(.caption).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 2)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Fassung vom \(AblageAnzeigeText.datum(f.zeit))" + (f.istAktuell ? ", aktuell" : "") + (konflikt && f.istKopf ? ", offene Fassung im Konflikt" : "")
                            + ", " + textStand(f.text) + (f.originalVorhanden ? ", Original geladen" : ""))
        .accessibilityIdentifier("ablageFassung")
    }

    private func textStand(_ s: AblageTextStand) -> String {
        switch s {
        case .bereit: return "Text vorhanden"
        case .keinText: return "kein Text"
        case .fehler: return "Text nicht erstellbar"
        case .nichtGeladen: return "Text noch nicht geladen"
        case .wartetAufPC: return "Text wird am PC erstellt"
        case .keinFormat: return "kein Text für dieses Format"
        }
    }
}

// MARK: - Vorschau des Originals (Quick Look des Systems)

/// Das geladene Original und der Name, den die Vorschau anzeigt (die Datei heißt auf der Platte nach ihrer Kennung).
struct AblageVorschauZiel: Identifiable {
    let url: URL
    let titel: String
    var id: URL { url }
}

/// Quick Look zeigt das Original; das iPhone liest oder deutet es nie selbst (kein Extrahieren, Spec §4.1).
struct AblageVorschau: UIViewControllerRepresentable {
    let ziel: AblageVorschauZiel
    let schliessen: () -> Void

    func makeCoordinator() -> Koordinator { Koordinator(ziel) }

    /// In einer Navigationsleiste mit dem Namen der Datei als Titel und „Fertig“ (Quick Look zeigt sonst den Dateinamen auf der
    /// Platte, der nach der Kennung heißt).
    func makeUIViewController(context: Context) -> UINavigationController {
        let c = QLPreviewController()
        c.dataSource = context.coordinator
        c.navigationItem.leftBarButtonItem = UIBarButtonItem(title: "Fertig", primaryAction: UIAction { _ in schliessen() })
        return UINavigationController(rootViewController: c)
    }

    func updateUIViewController(_ controller: UINavigationController, context: Context) {}

    final class Koordinator: NSObject, QLPreviewControllerDataSource {
        private let ziel: AblageVorschauZiel
        init(_ ziel: AblageVorschauZiel) { self.ziel = ziel }

        func numberOfPreviewItems(in controller: QLPreviewController) -> Int { 1 }

        func previewController(_ controller: QLPreviewController, previewItemAt index: Int) -> QLPreviewItem {
            Element(url: ziel.url, titel: ziel.titel)
        }
    }

    private final class Element: NSObject, QLPreviewItem {
        let url: URL
        let titel: String
        init(url: URL, titel: String) { self.url = url; self.titel = titel }
        var previewItemURL: URL? { url }
        var previewItemTitle: String? { titel }
    }
}

// MARK: - Textansicht mit Sprung zum Anker

struct AblageTextAnsicht: View {
    let dat: String
    let name: String
    let anker: String?
    @ObservedObject var ablage = AblageSync.shared
    @State private var abschnitte: [AblageAbschnitt]?
    @State private var geladen = false

    private func istZiel(_ a: AblageAbschnitt) -> Bool { anker.map { AblageBytes.equal($0, a.anker) } ?? false }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if let ab = abschnitte {
                        Text("Textfassung, am PC erstellt").font(.footnote).foregroundStyle(VoicePalette.secondaryText)
                        ForEach(Array(ab.enumerated()), id: \.offset) { i, a in
                            abschnitt(a, ziel: istZiel(a)).id(i)
                        }
                    } else if geladen {
                        ContentUnavailableView("Kein Text vorhanden", systemImage: "doc.text",
                                               description: Text("Der Text dieser Datei liegt hier nicht oder ist nicht lesbar."))
                    } else {
                        ProgressView()
                    }
                }.padding(16)
            }
            .background(VoicePalette.background)
            .task {
                abschnitte = await ablage.textfassung(dat: dat)
                geladen = true
                if let ab = abschnitte, let i = ab.firstIndex(where: istZiel) {
                    try? await Task.sleep(nanoseconds: 150_000_000)
                    withAnimation { proxy.scrollTo(i, anchor: .top) }
                }
            }
        }
        .navigationTitle(name)
        .navigationBarTitleDisplayMode(.inline)
    }

    private func abschnitt(_ a: AblageAbschnitt, ziel: Bool) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 8) {
                Text(AblageAnker.beschreibung(a.anker)).font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.secondaryText)
                if ziel { Label("Fundstelle", systemImage: "scope").font(.caption.weight(.semibold)).foregroundStyle(VoicePalette.text) }
            }
            if !a.pfad.isEmpty { Text(a.pfad.joined(separator: " › ")).font(.caption).foregroundStyle(VoicePalette.secondaryText).fixedSize(horizontal: false, vertical: true) }
            Text(a.text).font(.body).foregroundStyle(VoicePalette.text).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
        }
        .padding(12).frame(maxWidth: .infinity, alignment: .leading)
        .background(ziel ? VoicePalette.brand.opacity(0.28) : Color.clear, in: RoundedRectangle(cornerRadius: 8))
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(ziel ? VoicePalette.accent : VoicePalette.border, lineWidth: ziel ? 2 : 1))
        .accessibilityElement(children: .combine)
        .accessibilityLabel((ziel ? "Fundstelle. " : "") + AblageAnker.beschreibung(a.anker) + ". " + a.text)
        .accessibilityIdentifier(ziel ? "ablageZielabschnitt" : "ablageAbschnitt")
    }
}
