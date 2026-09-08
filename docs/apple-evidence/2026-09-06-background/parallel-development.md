# Parallele Entwicklung ohne verlorene Änderungen

Stand: 06.09.2026. Codex arbeitet ausschließlich im separaten Worktree
`/Users/patrick/Documents/Codex/local-voice-ai-apple-p0-p1` auf
`codex/watch-conversation-background`.

Der bisherige Codex-Stand bleibt auf `codex/apple-p0-p1` bei `d449e3e` erhalten.
Der neue Branch enthält dessen gesamte Entwicklung und die folgenden Gesprächsänderungen.
Der ursprüngliche Checkout `/Users/patrick/claude/local-voice-ai` bleibt auf `main`;
dessen lokale Desktop-/TTS-/Übersetzungsänderungen wurden nicht übernommen oder bearbeitet.

## Vorgehen

- Pro Entwickler ein eigener Branch und Worktree; keine gemeinsame Arbeitskopie für gleichzeitige Änderungen.
- Vor jedem Commit Branch, Status und Diff prüfen; nur konkret bearbeitete Dateien aufnehmen.
- Kleine additive Commits; keine fremden Commits umschreiben, kein Reset, kein automatisches Stash und kein Force-Push.
- Vor Integration Zielstand erneut lesen. Integration in einem separaten sauberen Integrations-Worktree prüfen; Konflikte je Datei fachlich lösen, niemals pauschal mit „ours“ oder „theirs“.
- Nach einer Konfliktauflösung die betroffenen Tests und Builds frisch ausführen. Dieser Arbeitsstand wird weder automatisch nach main gemergt noch veröffentlicht.

Branches verhindern das Überschreiben derselben Arbeitsdateien. Sie garantieren nicht,
dass sich spätere fachliche Änderungen konfliktfrei zusammenführen lassen. Die getrennten
Commits erhalten beide Entwicklungsstände für diese Prüfung.
