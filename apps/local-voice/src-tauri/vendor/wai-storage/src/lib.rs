//! wai-storage: SQLite ueber sqlx mit versionierten Migrationen, Backup/Restore
//! (`VACUUM INTO` + SHA-256-Manifest) und Erkennung korrupter Datenbanken.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub use sqlx;

/// Eine vorwaerts gerichtete, nummerierte Migration.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: i64,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Datenbank ist korrupt: {0}")]
    Corrupt(String),
    #[error("Datentraeger voll: {0}")]
    DiskFull(String),
    #[error("Migration {version} ({name}) fehlgeschlagen: {source}")]
    Migration {
        version: i64,
        name: String,
        source: sqlx::Error,
    },
    #[error("Migrationsliste ungueltig: {0}")]
    InvalidMigrations(String),
    #[error("Datenbank hat Schema-Version {found}, dieses Programm kennt nur bis {known}")]
    SchemaTooNew { found: i64, known: i64 },
    #[error(
        "Datenbank hat Schema-Version {found}, erwartet {expected} (nur lesend geoeffnet, keine Migration)"
    )]
    SchemaMismatch { found: i64, expected: i64 },
    #[error("Backup-Pruefsumme stimmt nicht: erwartet {expected}, gefunden {actual}")]
    HashMismatch { expected: String, actual: String },
    #[error("Manifest fehlt oder unlesbar: {0}")]
    Manifest(String),
    #[error("Ziel existiert bereits: {0}")]
    TargetExists(PathBuf),
    #[error("E/A-Fehler: {0}")]
    Io(std::io::Error),
    #[error("Datenbankfehler: {0}")]
    Db(sqlx::Error),
}

impl From<std::io::Error> for StorageError {
    fn from(e: std::io::Error) -> Self {
        classify_io(e)
    }
}

impl From<sqlx::Error> for StorageError {
    fn from(e: sqlx::Error) -> Self {
        classify_sqlx(e)
    }
}

pub type Result<T> = std::result::Result<T, StorageError>;

pub fn classify_io(e: std::io::Error) -> StorageError {
    // 28 = ENOSPC (Unix), 112 = ERROR_DISK_FULL, 39 = ERROR_HANDLE_DISK_FULL (Windows)
    if matches!(e.kind(), std::io::ErrorKind::StorageFull)
        || matches!(e.raw_os_error(), Some(28 | 112 | 39))
    {
        StorageError::DiskFull(e.to_string())
    } else {
        StorageError::Io(e)
    }
}

pub fn classify_sqlx(e: sqlx::Error) -> StorageError {
    if let sqlx::Error::Database(d) = &e {
        let code = d.code().map(|c| c.to_string()).unwrap_or_default();
        let msg = d.message().to_string();
        // primaere Codes: 13 SQLITE_FULL, 11 SQLITE_CORRUPT, 26 SQLITE_NOTADB (erweiterte: n & 0xff)
        if let Ok(n) = code.parse::<i64>() {
            match n & 0xff {
                13 => return StorageError::DiskFull(msg),
                11 | 26 => return StorageError::Corrupt(msg),
                _ => {}
            }
        }
        let lower = msg.to_lowercase();
        if lower.contains("database or disk is full") {
            return StorageError::DiskFull(msg);
        }
        if lower.contains("malformed") || lower.contains("not a database") {
            return StorageError::Corrupt(msg);
        }
    }
    StorageError::Db(e)
}

/// Fehler beim Oeffnen/Pruefen: alles, was kein Platzproblem ist, gilt als Korruption.
fn open_error(e: sqlx::Error) -> StorageError {
    match classify_sqlx(e) {
        StorageError::Db(e) => StorageError::Corrupt(e.to_string()),
        other => other,
    }
}

/// Dateisystem-Zugriff fuer Backup/Restore; ersetzbar zur Fehlerinjektion (z. B. volles Laufwerk).
pub trait Io: Send + Sync {
    fn copy(&self, src: &mut dyn Read, dst: &mut dyn Write) -> std::io::Result<u64>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StdIo;

impl Io for StdIo {
    fn copy(&self, src: &mut dyn Read, dst: &mut dyn Write) -> std::io::Result<u64> {
        std::io::copy(src, dst)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupManifest {
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    pub schema_version: i64,
}

fn manifest_path(backup: &Path) -> PathBuf {
    let mut s = backup.as_os_str().to_owned();
    s.push(".manifest.json");
    PathBuf::from(s)
}

pub fn sha256_file(path: &Path) -> Result<String> {
    sha256_reader(&mut std::fs::File::open(path)?)
}

fn sha256_reader(r: &mut impl Read) -> Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn options(path: &Path) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5))
        .foreign_keys(true)
}

/// Pruefung per `PRAGMA integrity_check`; nicht "ok" oder nicht lesbar = korrupt.
async fn check_integrity(pool: &SqlitePool) -> Result<()> {
    let rows: std::result::Result<Vec<String>, sqlx::Error> =
        sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_all(pool)
            .await;
    match rows {
        Ok(r) if r.len() == 1 && r[0] == "ok" => Ok(()),
        Ok(r) => Err(StorageError::Corrupt(r.join("; "))),
        Err(e) => Err(open_error(e)),
    }
}

#[derive(Debug, Clone)]
pub struct Storage {
    pool: SqlitePool,
    path: PathBuf,
}

impl Storage {
    /// Oeffnet (und legt ggf. an) die Datenbank, prueft Integritaet und migriert vorwaerts.
    /// Vor einer Migration einer bestehenden DB entsteht `<db>.pre-migration-v<alt>.bak`.
    pub async fn open(path: impl AsRef<Path>, migrations: &[Migration]) -> Result<Self> {
        validate(migrations)?;
        let path = path.as_ref().to_path_buf();
        // B42: Integritaetspruefung und Migration laufen auf genau EINER Verbindung; der
        // Arbeits-Pool entsteht erst danach frisch. So haelt keine Pool-Verbindung ein Schema
        // aus der Zeit vor einer ALTER-Migration (sonst: sqlx-Panic "len is N but index is N").
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options(&path))
            .await
            .map_err(open_error)?;
        let s = Storage { pool, path };
        check_integrity(&s.pool).await?;
        s.ensure_meta().await?;
        let current = s.schema_version().await?;
        let known = migrations.last().map(|m| m.version).unwrap_or(0);
        if current > known {
            return Err(StorageError::SchemaTooNew {
                found: current,
                known,
            });
        }
        let pending: Vec<&Migration> = migrations.iter().filter(|m| m.version > current).collect();
        if !pending.is_empty() {
            if current > 0 {
                let bak =
                    PathBuf::from(format!("{}.pre-migration-v{current}.bak", s.path.display()));
                if let Ok((_, d, n)) = open_parent(&bak) {
                    // remove_file entfernt auch einen Symlink selbst, nie dessen Ziel.
                    let _ = d.remove_file(&n);
                    let _ = d.remove_file(manifest_name(&n));
                }
                s.backup_to(&bak, &StdIo).await?;
            }
            for m in pending {
                s.apply(m).await?;
            }
        }
        let Storage { pool, path } = s;
        pool.close().await;
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options(&path))
            .await
            .map_err(open_error)?;
        Ok(Storage { pool, path })
    }

    /// Oeffnet eine bestehende Datenbank nur lesend, ohne Integritaetspruefung und ohne Migration: fuer
    /// Beobachter (z. B. eine Oberflaeche), waehrend ein anderer Prozess die Datei besitzt und migriert.
    /// Weicht die Schema-Version vom Stand der Migrationsliste ab, scheitert das Oeffnen mit
    /// [`StorageError::SchemaMismatch`], statt fremde Daten mit einem anderen Schema zu lesen.
    pub async fn open_read_only(path: impl AsRef<Path>, migrations: &[Migration]) -> Result<Self> {
        validate(migrations)?;
        let path = path.as_ref().to_path_buf();
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options(&path).create_if_missing(false).read_only(true))
            .await
            .map_err(open_error)?;
        let s = Storage { pool, path };
        let found = s.schema_version().await?;
        let expected = migrations.last().map(|m| m.version).unwrap_or(0);
        if found != expected {
            s.pool.close().await;
            return Err(StorageError::SchemaMismatch { found, expected });
        }
        Ok(s)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    async fn ensure_meta(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS _wai_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL DEFAULT (datetime('now')))",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn schema_version(&self) -> Result<i64> {
        let v: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _wai_migrations")
            .fetch_one(&self.pool)
            .await?;
        Ok(v.unwrap_or(0))
    }

    /// Jede Migration laeuft in einer Transaktion samt Versionseintrag: ganz oder gar nicht.
    async fn apply(&self, m: &Migration) -> Result<()> {
        let wrap = |source| StorageError::Migration {
            version: m.version,
            name: m.name.to_string(),
            source,
        };
        let mut tx = self.pool.begin().await.map_err(wrap)?;
        for stmt in m.sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::query(sqlx::AssertSqlSafe(stmt))
                .execute(&mut *tx)
                .await
                .map_err(wrap)?;
        }
        sqlx::query("INSERT INTO _wai_migrations (version, name) VALUES (?, ?)")
            .bind(m.version)
            .bind(m.name)
            .execute(&mut *tx)
            .await
            .map_err(wrap)?;
        tx.commit().await.map_err(wrap)?;
        Ok(())
    }

    /// Konsistente Sicherung per `VACUUM INTO`, danach Hash + Manifest.
    pub async fn backup(&self, dest: impl AsRef<Path>) -> Result<BackupManifest> {
        self.backup_to(dest.as_ref(), &StdIo).await
    }

    pub async fn backup_to(&self, dest: &Path, io: &dyn Io) -> Result<BackupManifest> {
        let (dir_path, dir, name) = open_parent(dest)?;
        if dir.symlink_metadata(&name).is_ok() {
            return Err(StorageError::TargetExists(dest.to_path_buf()));
        }
        // Exklusiv (O_EXCL / CREATE_NEW) angelegte, leere Tempdatei im Dir-Handle; VACUUM INTO
        // darf in eine leere vorhandene Datei schreiben.
        let (tmp_name, tmp_file) = create_exclusive_temp(&dir, &name, "part")?;
        drop(tmp_file);
        let res = sqlx::query("VACUUM INTO ?")
            .bind(dir_path.join(&tmp_name).to_string_lossy().to_string())
            .execute(&self.pool)
            .await;
        if let Err(e) = res {
            let _ = dir.remove_file(&tmp_name);
            return Err(classify_sqlx(e));
        }
        let mut src = match dir.open(&tmp_name) {
            Ok(f) => f,
            Err(e) => {
                let _ = dir.remove_file(&tmp_name);
                return Err(classify_io(e));
            }
        };
        let hash = sha256_reader(&mut src)?;
        let bytes = src.metadata()?.len();
        src.seek(SeekFrom::Start(0))?;
        // Ziel ohne Ersetzen anlegen: create_new schlaegt bei Datei UND Symlink fehl.
        let mut out = match create_new_private(&dir, &name) {
            Ok(f) => f,
            Err(e) => {
                let _ = dir.remove_file(&tmp_name);
                return Err(exists_or_io(e, dest));
            }
        };
        let copied = io.copy(&mut src, &mut out);
        drop(out);
        drop(src);
        let _ = dir.remove_file(&tmp_name);
        if let Err(e) = copied {
            let _ = dir.remove_file(&name);
            return Err(classify_io(e));
        }
        let actual = sha256_reader(&mut dir.open(&name)?)?;
        if actual != hash {
            let _ = dir.remove_file(&name);
            return Err(StorageError::HashMismatch {
                expected: hash,
                actual,
            });
        }
        let manifest = BackupManifest {
            file: name.to_string_lossy().into_owned(),
            sha256: hash,
            bytes,
            schema_version: self.schema_version().await?,
        };
        let json = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| StorageError::Manifest(e.to_string()))?;
        let mut mf = create_new_private(&dir, &manifest_name(&name))
            .map_err(|e| exists_or_io(e, &manifest_path(dest)))?;
        mf.write_all(&json)?;
        Ok(manifest)
    }

    /// Pflichtprobe: Backup pruefen (Hash, Integritaet) und in eine FRISCHE DB zurueckspielen.
    pub async fn restore(
        backup: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<BackupManifest> {
        Self::restore_with(backup.as_ref(), target.as_ref(), &StdIo).await
    }

    pub async fn restore_with(backup: &Path, target: &Path, io: &dyn Io) -> Result<BackupManifest> {
        let (_, bdir, bname) = open_parent(backup)?;
        let (tdir_path, tdir, tname) = open_parent(target)?;
        if tdir.symlink_metadata(&tname).is_ok() {
            return Err(StorageError::TargetExists(target.to_path_buf()));
        }
        let mut raw = Vec::new();
        bdir.open(manifest_name(&bname))
            .and_then(|mut f| f.read_to_end(&mut raw))
            .map_err(|e| StorageError::Manifest(e.to_string()))?;
        let manifest: BackupManifest =
            serde_json::from_slice(&raw).map_err(|e| StorageError::Manifest(e.to_string()))?;
        let mut src = bdir.open(&bname)?;
        let actual = sha256_reader(&mut src)?;
        if actual != manifest.sha256 {
            return Err(StorageError::HashMismatch {
                expected: manifest.sha256,
                actual,
            });
        }
        src.seek(SeekFrom::Start(0))?;
        let (tmp_name, mut tmp_file) = create_exclusive_temp(&tdir, &tname, "restoring")?;
        let copied = io.copy(&mut src, &mut tmp_file);
        drop(src);
        if let Err(e) = copied {
            drop(tmp_file);
            let _ = tdir.remove_file(&tmp_name);
            return Err(classify_io(e));
        }
        let _ = tmp_file.flush();
        drop(tmp_file);
        let tmp_path = tdir_path.join(&tmp_name);
        let verify = async {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(
                    SqliteConnectOptions::new()
                        .filename(&tmp_path)
                        .read_only(true),
                )
                .await
                .map_err(open_error)?;
            let r = check_integrity(&pool).await;
            pool.close().await;
            r
        };
        if let Err(e) = verify.await {
            let _ = tdir.remove_file(&tmp_name);
            return Err(e);
        }
        let after = match tdir.open(&tmp_name).map_err(StorageError::from) {
            Ok(mut f) => sha256_reader(&mut f),
            Err(e) => Err(e),
        };
        let after = match after {
            Ok(h) => h,
            Err(e) => {
                let _ = tdir.remove_file(&tmp_name);
                return Err(e);
            }
        };
        if after != manifest.sha256 {
            let _ = tdir.remove_file(&tmp_name);
            return Err(StorageError::HashMismatch {
                expected: manifest.sha256,
                actual: after,
            });
        }
        // Hardlink ersetzt nie ein vorhandenes Ziel (auch keinen Symlink) und ist atomar.
        let linked = tdir.hard_link(&tmp_name, &tdir, &tname);
        let _ = tdir.remove_file(&tmp_name);
        linked.map_err(|e| exists_or_io(e, target))?;
        Ok(manifest)
    }

    pub async fn close(self) {
        self.pool.close().await;
    }
}

/// Oeffnet das Zielverzeichnis EINMAL als Handle; alle weiteren Zugriffe sind relativ dazu
/// (cap-std verweigert Ausbruch per `..`/Symlink). Liefert Verzeichnispfad (nur fuer SQLite),
/// Handle und den einzelnen Dateinamen.
fn open_parent(path: &Path) -> Result<(PathBuf, Dir, std::ffi::OsString)> {
    let name = path
        .file_name()
        .ok_or_else(|| StorageError::Io(std::io::Error::other("Pfad ohne Dateinamen")))?
        .to_owned();
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let dir = Dir::open_ambient_dir(&parent, ambient_authority())?;
    Ok((parent, dir, name))
}

fn manifest_name(name: &std::ffi::OsStr) -> std::ffi::OsString {
    let mut s = name.to_owned();
    s.push(".manifest.json");
    s
}

/// Neue Datei, die nie eine vorhandene Datei/einen Symlink oeffnet oder ersetzt (Unix 0600).
fn create_new_private(dir: &Dir, name: &std::ffi::OsStr) -> std::io::Result<cap_std::fs::File> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    dir.open_with(name, &o)
}

fn exists_or_io(e: std::io::Error, path: &Path) -> StorageError {
    if e.kind() == std::io::ErrorKind::AlreadyExists {
        StorageError::TargetExists(path.to_path_buf())
    } else {
        classify_io(e)
    }
}

/// Zufaelliger (64 Bit) Tempname neben `base`, exklusiv im Dir-Handle angelegt.
/// Vorhandene Dateien werden nie geoeffnet, verfolgt oder geloescht.
fn create_exclusive_temp(
    dir: &Dir,
    base: &std::ffi::OsStr,
    suffix: &str,
) -> Result<(std::ffi::OsString, cap_std::fs::File)> {
    for _ in 0..8 {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r)
            .map_err(|e| StorageError::Io(std::io::Error::other(e.to_string())))?;
        let hex: String = r.iter().map(|b| format!("{b:02x}")).collect();
        let mut n = base.to_owned();
        n.push(format!(".{hex}.{suffix}"));
        match create_new_private(dir, &n) {
            Ok(f) => return Ok((n, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(classify_io(e)),
        }
    }
    Err(StorageError::Io(std::io::Error::other(
        "kein freier Tempname gefunden",
    )))
}

fn validate(migrations: &[Migration]) -> Result<()> {
    let mut prev = 0;
    for m in migrations {
        if m.version <= prev {
            return Err(StorageError::InvalidMigrations(format!(
                "Version {} nicht streng aufsteigend nach {prev}",
                m.version
            )));
        }
        prev = m.version;
    }
    Ok(())
}

/// Begrenzt die DB-Groesse (SQLITE_FULL) - simuliert ein volles Laufwerk fuer Tests.
pub async fn limit_pages(pool: &SqlitePool, pages: i64) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "PRAGMA max_page_count = {pages}"
    )))
    .execute(pool)
    .await?;
    Ok(())
}
