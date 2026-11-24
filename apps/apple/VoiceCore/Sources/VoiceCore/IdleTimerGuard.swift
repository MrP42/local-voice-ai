import Foundation

/// Gemeinsame Sperre gegen das Abdunkeln des Displays (lva#799, Codex-Review).
///
/// Mehrere Stellen der App wollen das Display wach halten: der Player („Bildschirm anlassen“), die
/// Live-Aufnahme, später weitere. Setzt jede den System-Schalter selbst, hebt die erste, die fertig
/// ist, die Sperre der anderen auf. Deshalb zählt dieser Wächter die Anforderungen: Das Display bleibt
/// wach, solange mindestens eine offen ist. Wer nicht über den Wächter geht, darf den Schalter nicht
/// mehr setzen.
@MainActor
public final class IdleTimerGuard {
    /// Anforderung eines Nutzers. Freigeben über ``release(_:)``; doppelte Freigabe ist harmlos.
    public struct Token: Hashable, Sendable {
        fileprivate let id: UInt64
        public let reason: String
    }

    private var open: Set<Token> = []
    private var next: UInt64 = 0
    private var applied: Bool?
    private let apply: (Bool) -> Void

    /// `apply` setzt den System-Schalter (auf dem iPhone `UIApplication.shared.isIdleTimerDisabled`).
    /// Er wird nur bei einer Änderung aufgerufen.
    public init(apply: @escaping (Bool) -> Void) {
        self.apply = apply
    }

    /// Hält das Display wach, bis das Token freigegeben ist.
    public func acquire(_ reason: String) -> Token {
        next += 1
        let token = Token(id: next, reason: reason)
        open.insert(token)
        update()
        return token
    }

    /// Gibt eine Anforderung frei. Erst wenn keine mehr offen ist, darf das Display wieder abdunkeln.
    public func release(_ token: Token?) {
        guard let token, open.remove(token) != nil else { return }
        update()
    }

    /// Display wird gerade wach gehalten.
    public var isActive: Bool { !open.isEmpty }

    /// Offene Anforderungen (für Diagnose und Tests).
    public var reasons: [String] { open.sorted { $0.id < $1.id }.map(\.reason) }

    private func update() {
        let want = !open.isEmpty
        guard applied != want else { return }
        applied = want
        apply(want)
    }
}
