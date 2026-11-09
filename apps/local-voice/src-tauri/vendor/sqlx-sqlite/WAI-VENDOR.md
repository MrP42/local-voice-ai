# sqlx-sqlite 0.9.0 (vendored, eine Zeile geändert)

**Warum:** RUSTSEC-2026-0459 / CVE-2026-119375 – das gebündelte SQLite < 3.54 in
`libsqlite3-sys` < 0.39. Behoben erst ab `libsqlite3-sys` 0.39 (SQLite 3.54, `rusqlite` 0.41).
`sqlx-sqlite` 0.9.0 (crates.io) erlaubt aber nur `libsqlite3-sys >=0.30.1, <0.38.0`; wegen
`links = "sqlite3"` darf es im Graphen nur eine `libsqlite3-sys` geben.

**Was:** Unveränderter Inhalt von `sqlx-sqlite-0.9.0.crate` (crates.io, VCS-Stand
`003b698e99e024f3621b8043a2426fde5b741171`). Einzige Änderung: die Obergrenze von
`libsqlite3-sys` in `Cargo.toml` und `Cargo.toml.orig` von `<0.38.0` auf `<0.40.0`.
Das entspricht genau den Upstream-Commits
[`94aafe3a`](https://github.com/launchbadge/sqlx/commit/94aafe3a68884d923b0798a767c8d7f6cfda89d2)
(#4402, 0.38.x) und
[`10462fa4`](https://github.com/launchbadge/sqlx/commit/10462fa4414c6c7f1ed1d39bb31678e3b726878c)
(#4502, 0.39.x); beide ändern nur diese Zeile.

**Eingebunden über** `[patch.crates-io]` in `../../Cargo.toml` (`sqlx` kommt über `vendor/wai-storage`).
Gleiche Kopie wie `wai-sdk/vendor/sqlx-sqlite`.

**Entfernen, sobald** ein `sqlx`-Release mit `libsqlite3-sys` 0.39 auf crates.io liegt:
Ordner löschen, den `[patch.crates-io] sqlx-sqlite`-Eintrag entfernen, `wai-storage` neu vendoren.

Lizenz: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
