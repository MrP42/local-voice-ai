import Foundation
import XCTest
@testable import VoiceCore

extension AssistantTurn {
    /// Der Beitrag mit gespeicherter, vollständig markierter Einheit (Runde 8, R1/U2): Ein Beitrag ohne Abschnitte (ältere Dateien)
    /// zeigt das Gespräch nie und gibt ihn nie in einen Prompt. Tests, die einen Beitrag anzeigen oder in den Prompt geben lassen,
    /// bauen ihn so; `parts` bleibt unberührt, wo es der Test selbst setzt.
    var shown: AssistantTurn {
        guard parts.isEmpty else { return self }
        var copy = self
        copy.parts = [AssistantTurnPart(text: assistant, unsourced: false, note: AssistantTurnPart.templateNote)]
        return copy
    }
}
