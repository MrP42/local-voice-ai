import SwiftUI

/// Schrift und Farbe der Leseansicht, wie am Desktop (`reader/ReaderToolbar.tsx`, `reader.css`):
/// drei Größen, drei Schriften, fünf Farbthemen mit denselben Farben. Gilt für Vorlesen und Mitlesen.
@MainActor
final class ReaderAppearance: ObservableObject {
    static let shared = ReaderAppearance()

    enum Face: String, CaseIterable, Identifiable {
        case book, sans, wide
        var id: String { rawValue }
        var title: String { self == .book ? "Buchschrift" : self == .sans ? "Serifenlos" : "Breite Schrift" }
    }
    enum Size: Int, CaseIterable, Identifiable {
        case small = 16, medium = 18, large = 21
        var id: Int { rawValue }
        var title: String { self == .small ? "Klein" : self == .medium ? "Mittel" : "Groß" }
    }
    enum Theme: String, CaseIterable, Identifiable {
        case auto, white, sepia, green, black
        var id: String { rawValue }
        var title: String {
            switch self {
            case .auto: "Wie die App"
            case .white: "Weiß"
            case .sepia: "Sepia"
            case .green: "Grün"
            case .black: "Schwarz"
            }
        }
    }

    @Published var face: Face { didSet { UserDefaults.standard.set(face.rawValue, forKey: "reader.font") } }
    @Published var size: Size { didSet { UserDefaults.standard.set(size.rawValue, forKey: "reader.size") } }
    @Published var theme: Theme { didSet { UserDefaults.standard.set(theme.rawValue, forKey: "reader.theme") } }

    init() {
        let d = UserDefaults.standard
        face = d.string(forKey: "reader.font").flatMap(Face.init) ?? .book
        size = Size(rawValue: d.integer(forKey: "reader.size")) ?? .medium
        theme = d.string(forKey: "reader.theme").flatMap(Theme.init) ?? .auto
    }

    private static func hex(_ v: Int) -> Color {
        Color(red: Double((v >> 16) & 255) / 255, green: Double((v >> 8) & 255) / 255, blue: Double(v & 255) / 255)
    }

    private var colors: (background: Int, text: Int, muted: Int)? {
        switch theme {
        case .auto: nil
        case .white: (0xFFFFFF, 0x1B1B1B, 0x4A4A4A)
        case .sepia: (0xF4ECD8, 0x3B2F1E, 0x5C4A33)
        case .green: (0xD5EAD2, 0x1F2D1F, 0x334A35)
        case .black: (0x000000, 0xE6E6E6, 0xABABAB)
        }
    }

    var background: Color { colors.map { Self.hex($0.background) } ?? VoicePalette.background }
    var text: Color { colors.map { Self.hex($0.text) } ?? VoicePalette.text }
    var muted: Color { colors.map { Self.hex($0.muted) } ?? VoicePalette.secondaryText }
    /// Aktuelles Wort: auf Schwarz wäre Tinte unsichtbar.
    /// Heller Ton hinter dem gelesenen Satz; die Schrift bleibt die des Lesethemas.
    var sentenceBackground: Color { VoicePalette.brand.opacity(theme == .black ? 0.28 : 0.38) }
    /// Gesprochenes Wort beim Mitlesen: kräftiger als der Satz. Nur Hintergrund, damit der Umbruch nicht springt.
    /// Schrift darauf ist immer Tinte, auch im Thema Schwarz (sonst hell auf hellem Grund).
    var wordBackground: Color { VoicePalette.brand.opacity(0.9) }
    var wordText: Color { VoicePalette.ink }

    /// Nicht `.system(design: .serif)`: SwiftUI misst New York unter iOS 26 falsch und schneidet Absätze ab.
    func font(heading: Bool) -> Font {
        let points = CGFloat(size.rawValue) + (heading ? 3 : 0)
        let style: Font.TextStyle = heading ? .title3 : .body
        switch face {
        case .book: return .custom(heading ? "Georgia-Bold" : "Georgia", size: points, relativeTo: style)
        case .sans: return .system(size: points, weight: heading ? .bold : .regular)
        case .wide: return .custom(heading ? "Verdana-Bold" : "Verdana", size: points, relativeTo: style)
        }
    }

    /// Entspricht `line-height: 1.72` der Leseansicht am Desktop.
    var lineSpacing: CGFloat { CGFloat(size.rawValue) * 0.5 }
}

/// Knopf „Aa“ für die Kopfzeile der Leseansicht.
struct ReaderAppearanceButton: View {
    @State private var shown = false
    var body: some View {
        Button { shown = true } label: {
            Text("Aa").font(.custom("Georgia", size: 19)).frame(width: 44, height: 44)
        }
        .accessibilityLabel("Schrift und Darstellung")
        .sheet(isPresented: $shown) { ReaderAppearanceSheet().presentationDetents([.medium]) }
    }
}

struct ReaderAppearanceSheet: View {
    @ObservedObject var look = ReaderAppearance.shared
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section("Größe") {
                    Picker("Größe", selection: $look.size) {
                        ForEach(ReaderAppearance.Size.allCases) { Text($0.title).tag($0) }
                    }.pickerStyle(.segmented).labelsHidden()
                }
                Section("Schriftart") {
                    ForEach(ReaderAppearance.Face.allCases) { face in
                        Button { look.face = face } label: {
                            HStack {
                                Text(face.title).font(fontSample(face))
                                Spacer()
                                if look.face == face { Image(systemName: "checkmark") }
                            }.frame(minHeight: 44).contentShape(Rectangle())
                        }.buttonStyle(.plain).accessibilityAddTraits(look.face == face ? .isSelected : [])
                    }
                }
                Section("Farbe") {
                    ForEach(ReaderAppearance.Theme.allCases) { theme in
                        Button { look.theme = theme } label: {
                            HStack {
                                Text(theme.title)
                                Spacer()
                                if look.theme == theme { Image(systemName: "checkmark") }
                            }.frame(minHeight: 44).contentShape(Rectangle())
                        }.buttonStyle(.plain).accessibilityAddTraits(look.theme == theme ? .isSelected : [])
                    }
                }
            }
            .navigationTitle("Darstellung").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Fertig") { dismiss() } } }
        }
    }

    private func fontSample(_ face: ReaderAppearance.Face) -> Font {
        switch face {
        case .book: .custom("Georgia", size: 17)
        case .sans: .system(size: 17)
        case .wide: .custom("Verdana", size: 16)
        }
    }
}
