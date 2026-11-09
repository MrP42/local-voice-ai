# rusqlite_migration 2.3.0 (vendored, eine Zeile geändert)

**Warum:** RUSTSEC-2026-0459 / CVE-2026-119375 – das gebündelte SQLite < 3.54 in
`libsqlite3-sys` < 0.39. Behoben erst ab `libsqlite3-sys` 0.39 (SQLite 3.54) mit `rusqlite` 0.41.
Kein veröffentlichtes `rusqlite_migration` erlaubt `rusqlite` 0.41 (2.6.0 und `master`
verlangen `^0.40`), und wegen `links = "sqlite3"` darf es nur eine `libsqlite3-sys` geben.

**Was:** Unveränderter Inhalt von `rusqlite_migration-2.3.0.crate` (crates.io, VCS-Stand
`10aca820577fc3c2fe1d54eb3dca07ab7e4e08bf`), also genau die bisher genutzte Version mit
gleichem Verhalten und gleichen Fehlertexten. Einzige Änderung: in `Cargo.toml`
`[dependencies.rusqlite] version` von `0.37.0` auf `0.41.0`. `Cargo.toml.orig` ist der
unveränderte Upstream-Stand und wird von Cargo nicht gelesen.

**Eingebunden über** die Pfad-Abhängigkeit `rusqlite_migration` in `../../Cargo.toml`.

**Entfernen, sobald** ein `rusqlite_migration`-Release mit `rusqlite` 0.41 auf crates.io liegt:
Ordner löschen, Abhängigkeit wieder auf die crates.io-Version stellen.

Lizenz: Apache-2.0 (Kopfzeilen in den Quellen, siehe `Cargo.toml`).
