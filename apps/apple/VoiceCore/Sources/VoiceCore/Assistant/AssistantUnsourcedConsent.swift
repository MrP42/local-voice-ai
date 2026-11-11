import Foundation

// Gesprächsmodus „Sprechen“ (Paket A, E4.2a): Einwilligung in „Antworten ohne Quelle“.
// Eigene Einwilligung, getrennt von den Freigaben für Cloud und Bereiche. Standard: aus. Sie wird nie durch eine Migration
// oder einen Standardwert eingeschaltet; nur `grant()` nach der Bestätigung im Menü setzt sie. Ändert sich der Text der
// Bestätigung (`currentVersion` hoch), gilt eine ältere Einwilligung nicht mehr und das Menü fragt neu.

public struct AssistantUnsourcedConsent {
    public static let currentVersion = 1
    public static let enabledKey = "assistant_unsourced_enabled"
    public static let versionKey = "assistant_unsourced_version"
    public static let dateKey = "assistant_unsourced_date"

    /// Text der Bestätigung beim ersten Einschalten (Fassung `currentVersion`).
    public static let confirmationText = "Der Assistent darf dann auch aus dem Wissen des Sprachmodells antworten. Solche Sätze sind nicht durch deine Inhalte belegt und können falsch sein. Sie werden vor dem Sprechen mit ‚Ohne Quelle‘ angesagt und im Verlauf gekennzeichnet. Du kannst das jederzeit ausschalten."

    private let defaults: UserDefaults

    public init(defaults: UserDefaults = .standard) { self.defaults = defaults }

    /// Ob der Assistent unbelegte Sätze sprechen darf. Aus, solange nichts gespeichert ist oder die Einwilligung zu einer
    /// anderen Fassung des Bestätigungstexts gehört.
    public var isEnabled: Bool {
        defaults.bool(forKey: Self.enabledKey) && defaults.integer(forKey: Self.versionKey) == Self.currentVersion
    }

    /// Version, mit der zuletzt eingewilligt wurde (0: nie).
    public var version: Int { defaults.integer(forKey: Self.versionKey) }

    /// Der aktuelle Bestätigungstext wurde schon einmal bestätigt (auch wenn der Schalter jetzt aus ist): dann schaltet das
    /// Menü ohne neue Rückfrage wieder ein. Ein Schalter ohne Version oder mit älterer Fassung gilt nicht als bestätigt.
    public var isAcknowledged: Bool { version == Self.currentVersion }

    public var grantedAt: Date? { defaults.object(forKey: Self.dateKey) as? Date }

    /// Nach der Bestätigung durch den Nutzer.
    public func grant(now: Date = Date()) {
        defaults.set(true, forKey: Self.enabledKey)
        defaults.set(Self.currentVersion, forKey: Self.versionKey)
        defaults.set(now, forKey: Self.dateKey)
    }

    public func revoke() {
        defaults.set(false, forKey: Self.enabledKey)
    }
}
