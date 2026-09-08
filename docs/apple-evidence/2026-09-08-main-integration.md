# Apple-Integration in main – 08.09.2026

Basis: origin/main 10a9309; Apple-Branch da9ca31. Separater Worktree und Branch
codex/integrate-apple-main. Originale Arbeitsverzeichnisse nicht geändert.

TTS-Konflikt inhaltlich gelöst: bestehende Auto-Transkription, Sprecherverwaltung,
Exportoptionen und alle TTS-Einstellungen aus main bleiben erhalten; die kompakte
Optionsgruppe wird darum gelegt. Apple-Plan übernimmt den dokumentierten
Umsetzungsfortschritt. Keine Änderung an Desktop-Rust-Backend, Cargo-Abhängigkeiten,
Desktop-Versionsnummer 0.16.0 oder bestehenden Release-Workflows.

Frisch erfolgreich: 89 Apple-Kerntests; Desktop TypeScript, Vite-Produktionsbuild,
ESLint und sechs Playwright-Workspace-Tests (inklusive Windows-Oberflächenmodus,
Hell/Dunkel, schmalem Fenster). Neue Stimmen-Metadatenantwort im UI-Testmock ergänzt.
Alle vorhandenen de/en-Übersetzungsschlüssel und verwendeten TTS-Einstellungsschlüssel
gegen main geprüft. Ein vollständiger nativer Windows-Installerbuild wurde hier
nicht durchgeführt; der native Desktop-Code ist identisch zum Main-Ausgangspunkt.

Apple: Release-Simulatorbuild und unsigned Device-Archive mit eingebetteter Watch
bestanden. Bisher fehlende ArchiveAction im Projektgenerator ergänzt, Watch
SKIP_INSTALL=YES und Phone SKIP_INSTALL=NO. Generierte Schemes reproduzierbar.
Apple validation prüft diese Pfade künftig auf GitHub. Das CI-Archiv ist kein
installierbares Release. Öffentliche Apple-Verteilung bleibt vom Developer-Program,
Distribution-Signing und App Store Connect abhängig; siehe apps/apple/RELEASING.md.
Kein Release-Tag, kein Upload zu App Store Connect und keine Veröffentlichung.

Offener PR #21 zur Strg-Taste bleibt unabhängig; kein fremder PR wird geschlossen
oder ungeprüft mitgemerged. Vor dem endgültigen Merge Main-Stand erneut abgleichen.
