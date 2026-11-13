#!/usr/bin/env python3
"""Erzeugt aus apps/shared/assistenten/vorlagen.json die beiden eingebetteten Fassungen der Assistenten-Vorlagen:

- apps/apple/VoiceCore/Sources/VoiceCore/Assistant/AssistantProfileTemplates.swift (iPhone)
- apps/local-voice/src/components/voice-call/assistenten/vorlagenJson.ts (PC; Vite darf nicht aus dem Ordner
  außerhalb von `apps/local-voice` laden, darum liegt der JSON-Text dort als Zeichenkette)

Aufruf (von ueberall):  python3 apps/apple/scripts/generate_assistant_templates.py [--check]
`--check` schreibt nichts und endet mit Code 1, wenn eine der Dateien nicht zur Quelle passt.
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
QUELLE = ROOT / "apps/shared/assistenten/vorlagen.json"
SWIFT = ROOT / "apps/apple/VoiceCore/Sources/VoiceCore/Assistant/AssistantProfileTemplates.swift"
TS = ROOT / "apps/local-voice/src/components/voice-call/assistenten/vorlagenJson.ts"

KOPF = (
    "ERZEUGT aus apps/shared/assistenten/vorlagen.json durch apps/apple/scripts/generate_assistant_templates.py"
    " - erzeugt, nicht von Hand ändern."
)

SWIFT_VORLAGE = '''// {kopf}
import Foundation

/// Die eingebauten Assistenten-Vorlagen (nur lesbar, nie im Speicherverzeichnis). `json` ist der Text von `vorlagen.json`
/// ohne den abschließenden Zeilenumbruch; ein Test hält ihn gleich der Quelle.
public enum AssistantProfileTemplates {{
    public static let json = #"""
{json}
"""#

    /// Alle Vorlagen in der Reihenfolge der Datei; die erste ist `alles`.
    public static let all: [AssistantProfile] = {{
        guard case .array(let items)? = AssistantJSON.parse(json) else {{ return [] }}
        return items.compactMap {{ item -> AssistantProfile? in
            if case .success(let profile) = AssistantProfile.parse(tree: item) {{ return profile }}
            return nil
        }}
    }}()

    /// Die Vorlage `alles` (Standard, wenn nichts anderes gewählt ist).
    public static var everything: AssistantProfile {{ all[0] }}

    public static func isTemplate(_ id: String) -> Bool {{
        all.contains {{ $0.id.utf8.elementsEqual(id.utf8) }}
    }}

    /// Vorbehalten für Vorlagen: ihre Ids und alles mit dem Vorsatz `vorlage-`.
    public static func isReservedId(_ id: String) -> Bool {{
        isTemplate(id) || id.utf8.starts(with: "vorlage-".utf8)
    }}

    public static func template(id: String) -> AssistantProfile? {{
        all.first {{ $0.id.utf8.elementsEqual(id.utf8) }}
    }}

    /// Eigene Kopie einer Vorlage mit neuer Id und neuem Namen; geprüft wie jede Eingabe. Eine vorbehaltene Id wird abgewiesen.
    public static func copyAsOwn(_ template: AssistantProfile, newId: String, newName: String) -> Result<AssistantProfile, AssistantProfileFailure> {{
        var copy = template
        copy.id = newId
        copy.name = newName
        var issues: [AssistantProfileIssue] = []
        switch AssistantProfile.parse(tree: copy.tree()) {{
        case .success(let profile):
            if !isReservedId(newId) {{ return .success(profile) }}
        case .failure(let failure):
            issues = failure.issues
        }}
        if isReservedId(newId) {{ issues.append(AssistantProfileIssue(path: "id", code: .value)) }}
        return .failure(AssistantProfileFailure(issues))
    }}
}}
'''

TS_VORLAGE = '''// {kopf}
export const VORLAGEN_JSON =
  {literal};
'''


def ts_literal(text: str) -> str:
    """JS-Zeichenkette in der Schreibweise, die Prettier wählt (einfache Anführungszeichen, wenn weniger Escapes nötig sind)."""
    inhalt = json.dumps(text, ensure_ascii=False)[1:-1]
    if inhalt.count('\\"') <= inhalt.count("'"):
        return '"' + inhalt + '"'
    return "'" + inhalt.replace('\\"', '"').replace("'", "\\'") + "'"


def main() -> int:
    text = QUELLE.read_text(encoding="utf-8")
    json.loads(text)  # muss gültiges JSON sein
    swift = SWIFT_VORLAGE.format(kopf=KOPF, json=text.rstrip("\n"))
    ts = TS_VORLAGE.format(kopf=KOPF, literal=ts_literal(text))
    if "--check" in sys.argv[1:]:
        stand = {
            SWIFT: SWIFT.read_text(encoding="utf-8") if SWIFT.exists() else None,
            TS: TS.read_text(encoding="utf-8") if TS.exists() else None,
        }
        veraltet = [str(p) for p, soll in ((SWIFT, swift), (TS, ts)) if stand[p] != soll]
        for p in veraltet:
            print("veraltet:", p, file=sys.stderr)
        return 1 if veraltet else 0
    SWIFT.write_text(swift, encoding="utf-8", newline="\n")
    TS.write_text(ts, encoding="utf-8", newline="\n")
    return 0


sys.exit(main())
