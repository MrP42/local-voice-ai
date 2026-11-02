//! `Projektverzeichnis`: SQLite ueber `wai-storage`, Mandant ausschliesslich aus dem `Principal`.

use std::path::Path;
use std::sync::Mutex;

use wai_security::Principal;
use wai_storage::sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow};
use wai_storage::sqlx::{self, Row, SqlitePool};
use wai_storage::{Migration, Storage};

use crate::fehler::{Ergebnis, ProjektFehler};
use crate::hilfen::{jetzt_ms, neue_id, neue_trace_id, slug_ok, wai_muster_ok, zeitstempel};
use crate::modell::*;

/// Hoechststand der Schemaversion, den diese Crate-Version kennt.
pub const SCHEMA_STAND: i64 = 1;

const MIGRATIONEN: &[Migration] = &[Migration {
    version: 1,
    name: "projekte_initial",
    sql: "CREATE TABLE projekte (
        id TEXT PRIMARY KEY, mandant TEXT NOT NULL, title TEXT NOT NULL,
        goal TEXT, description TEXT, instructions TEXT,
        data_class INTEGER NOT NULL, special_category INTEGER NOT NULL,
        sovereignty TEXT NOT NULL, retention_days INTEGER, web_research_enabled INTEGER,
        archived INTEGER NOT NULL DEFAULT 0, revision INTEGER NOT NULL,
        created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
    CREATE INDEX projekte_mandant ON projekte(mandant);
    CREATE TABLE plan_zuordnung (
        mandant TEXT NOT NULL, projekt_id TEXT NOT NULL REFERENCES projekte(id) ON DELETE CASCADE,
        plan_ref TEXT NOT NULL);
    CREATE UNIQUE INDEX plan_je_mandant ON plan_zuordnung(mandant, plan_ref);
    CREATE UNIQUE INDEX projekt_je_mandant ON plan_zuordnung(mandant, projekt_id);
    CREATE TABLE eintraege (
        id TEXT PRIMARY KEY, mandant TEXT NOT NULL,
        projekt_id TEXT NOT NULL REFERENCES projekte(id) ON DELETE CASCADE,
        kind TEXT NOT NULL, title TEXT NOT NULL, origin_app TEXT NOT NULL, ref TEXT NOT NULL,
        data_class INTEGER NOT NULL, assigned_by TEXT NOT NULL, assigned_ms INTEGER NOT NULL,
        preview TEXT, external_copies TEXT NOT NULL DEFAULT '[]');
    CREATE UNIQUE INDEX eintrag_ref_je_projekt ON eintraege(mandant, projekt_id, ref);
    CREATE TABLE loesch_outbox (
        id INTEGER PRIMARY KEY AUTOINCREMENT, mandant TEXT NOT NULL, projekt_id TEXT NOT NULL,
        eintrag_id TEXT, ref TEXT, grund TEXT NOT NULL, erzeugt_ms INTEGER NOT NULL);
    CREATE INDEX outbox_mandant ON loesch_outbox(mandant)",
}];

/// Rechtepruefung gegen die Herkunftsapp (ADR-0026 E3): wird bei Zuordnung UND bei jedem Auflisten gefragt.
pub trait Herkunftsrecht: Send + Sync {
    fn darf_lesen(&self, principal: &Principal, ref_: &str) -> bool;

    /// Datenklasse laut Herkunftsapp, falls bekannt. Massgeblich ist das Maximum aus Angabe der
    /// aufrufenden Seite und dieser Klasse; `None` = Angabe der aufrufenden Seite.
    fn klasse(&self, _principal: &Principal, _ref_: &str) -> Option<Klasse> {
        None
    }
}

/// Antwort auf die Frage, ob ein Ursprung noch existiert. `Unbekannt` (Herkunft nicht erreichbar,
/// Fehler) fuehrt nie zum Loeschen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vorhanden {
    Ja,
    Nein,
    Unbekannt,
}

/// Bestandsfrage fuer den Abgleich: existiert der Ursprung noch?
pub trait Herkunftsbestand: Send + Sync {
    fn existiert(&self, mandant: &str, ref_: &str) -> Vorhanden;
}

/// Obergrenze des Ereignis-Sammlers (Ringpuffer).
pub const MAX_EREIGNISSE: usize = 1000;

type Senke = Box<dyn Fn(&Ereignis) + Send + Sync>;

pub struct Projektverzeichnis {
    pool: SqlitePool,
    /// Gesetzt, wenn die Datei eine neuere Schemaversion hat: nur lesend.
    schema_neuer: Option<i64>,
    ereignisse: Mutex<Vec<Ereignis>>,
    senke: Option<Senke>,
}

fn mandant(p: &Principal) -> String {
    p.tenant().0.clone()
}

impl Projektverzeichnis {
    /// Oeffnet (und migriert) die Datei. Hat sie eine neuere Schemaversion, wird nur lesend geoeffnet.
    pub async fn oeffnen(pfad: impl AsRef<Path>) -> Ergebnis<Self> {
        let pfad = pfad.as_ref();
        if pfad.exists()
            && let Some(v) = vorhandene_version(pfad).await
            && v > SCHEMA_STAND
        {
            let pool = SqlitePoolOptions::new()
                .max_connections(2)
                .connect_with(SqliteConnectOptions::new().filename(pfad).read_only(true))
                .await?;
            return Ok(Self::aus(pool, Some(v)));
        }
        let storage = Storage::open(pfad, MIGRATIONEN).await?;
        Ok(Self::aus(storage.pool().clone(), None))
    }

    fn aus(pool: SqlitePool, schema_neuer: Option<i64>) -> Self {
        Projektverzeichnis {
            pool,
            schema_neuer,
            ereignisse: Mutex::new(Vec::new()),
            senke: None,
        }
    }

    /// Zusaetzlich zum Sammler jedes Ereignis an eine Funktion geben.
    pub fn mit_senke(mut self, f: impl Fn(&Ereignis) + Send + Sync + 'static) -> Self {
        self.senke = Some(Box::new(f));
        self
    }

    pub fn nur_lesend(&self) -> bool {
        self.schema_neuer.is_some()
    }

    /// Gesammelte Ereignisse abholen (leert den Sammler). Der Sammler haelt hoechstens
    /// `MAX_EREIGNISSE` Eintraege (aelteste fallen weg); dauerhaft ist nur die Outbox.
    pub fn ereignisse_abholen(&self) -> Vec<Ereignis> {
        std::mem::take(&mut *self.ereignisse.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn schreibbar(&self) -> Ergebnis<()> {
        match self.schema_neuer {
            Some(g) => Err(ProjektFehler::SchemaNeuer {
                gefunden: g,
                bekannt: SCHEMA_STAND,
            }),
            None => Ok(()),
        }
    }

    /// Schreibtransaktion mit `BEGIN IMMEDIATE`: Schreiber werden serialisiert. Ist die Datenbank
    /// belegt, wird mit Projektbezug `RevisionKonflikt` (mit der echten Revision) gemeldet.
    async fn schreib_tx(&self, m: &str, projekt: Option<&str>) -> Ergebnis<Tx<'static>> {
        match self.pool.begin_with("BEGIN IMMEDIATE").await {
            Ok(tx) => Ok(tx),
            Err(e) if ist_belegt(&e) => match projekt {
                Some(id) => Err(self.konflikt(m, id).await),
                None => Err(e.into()),
            },
            Err(e) => Err(e.into()),
        }
    }

    /// Aktuelle Revision nachlesen und als Konflikt melden.
    async fn konflikt(&self, m: &str, id: &str) -> ProjektFehler {
        let r: Result<Option<i64>, _> =
            sqlx::query_scalar("SELECT revision FROM projekte WHERE mandant=? AND id=?")
                .bind(m)
                .bind(id)
                .fetch_optional(&self.pool)
                .await;
        match r {
            Ok(Some(aktuell)) => ProjektFehler::RevisionKonflikt { aktuell },
            Ok(None) => ProjektFehler::NichtGefunden,
            Err(e) => e.into(),
        }
    }

    fn melden(&self, name: &'static str, projekt: &str, eintrag: Option<&str>, art: Option<Art>) {
        let e = Ereignis {
            name,
            projekt_id: projekt.into(),
            eintrag_id: eintrag.map(str::to_string),
            art,
        };
        if let Some(s) = &self.senke {
            s(&e);
        }
        let mut puffer = self.ereignisse.lock().unwrap_or_else(|x| x.into_inner());
        if puffer.len() >= MAX_EREIGNISSE {
            puffer.remove(0);
        }
        puffer.push(e);
    }

    /// Konsistente Sicherung per `VACUUM INTO`; das Ziel darf nicht existieren.
    pub async fn sichern(&self, ziel: impl AsRef<Path>) -> Ergebnis<()> {
        let ziel = ziel.as_ref();
        if ziel.exists() {
            return Err(ProjektFehler::Ungueltig(
                "Sicherungsziel existiert bereits".into(),
            ));
        }
        let text = ziel
            .to_str()
            .ok_or_else(|| ProjektFehler::Ungueltig("Pfad nicht als Text darstellbar".into()))?;
        sqlx::query("VACUUM INTO ?")
            .bind(text)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ------------------------------------------------------------------ Projekte

    pub async fn projekt_anlegen(&self, p: &Principal, neu: NeuesProjekt) -> Ergebnis<Projekt> {
        self.schreibbar()?;
        pruefe_felder(
            Some(&neu.title),
            neu.goal.as_deref(),
            neu.description.as_deref(),
            neu.instructions.as_deref(),
            neu.retention_days,
        )?;
        let plan = match &neu.plan_ref {
            Some(r) => Some(plan_pruefen(r)?),
            None => None,
        };
        let m = mandant(p);
        let id = neue_id("prj");
        let jetzt = jetzt_ms();
        let mut tx = self.schreib_tx(&m, None).await?;
        sqlx::query(
            "INSERT INTO projekte (id, mandant, title, goal, description, instructions, data_class,
             special_category, sovereignty, retention_days, web_research_enabled, archived, revision,
             created_ms, updated_ms) VALUES (?,?,?,?,?,?,?,?,?,?,?,0,1,?,?)",
        )
        .bind(&id)
        .bind(&m)
        .bind(&neu.title)
        .bind(&neu.goal)
        .bind(&neu.description)
        .bind(&neu.instructions)
        .bind(neu.data_class.zahl())
        .bind(neu.special_category)
        .bind(neu.sovereignty.text())
        .bind(neu.retention_days)
        .bind(neu.web_research_enabled)
        .bind(jetzt)
        .bind(jetzt)
        .execute(&mut *tx)
        .await?;
        if let Some(r) = &plan {
            plan_einfuegen(&mut tx, &m, &id, r).await?;
        }
        tx.commit().await?;
        self.melden("projekt.angelegt", &id, None, None);
        self.projekt_lesen(p, &id).await
    }

    pub async fn projekt_lesen(&self, p: &Principal, id: &str) -> Ergebnis<Projekt> {
        let row = sqlx::query(PROJEKT_SELECT_EINS)
            .bind(mandant(p))
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(|r| projekt_aus(&r))
            .ok_or(ProjektFehler::NichtGefunden)
    }

    pub async fn projekte_auflisten(
        &self,
        p: &Principal,
        mit_archivierten: bool,
    ) -> Ergebnis<Vec<Projekt>> {
        let rows = sqlx::query(PROJEKT_SELECT_ALLE)
            .bind(mandant(p))
            .bind(mit_archivierten)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.iter().map(projekt_aus).collect())
    }

    /// Aendert Felder; `erwartete_revision` muss der gelesenen entsprechen, sonst `RevisionKonflikt`.
    pub async fn projekt_aendern(
        &self,
        p: &Principal,
        id: &str,
        erwartete_revision: i64,
        a: Aenderung,
    ) -> Ergebnis<Projekt> {
        self.schreibbar()?;
        let m = mandant(p);
        let mut tx = self.schreib_tx(&m, Some(id)).await?;
        let alt = projekt_in_tx(&mut tx, &m, id).await?;
        if alt.revision != erwartete_revision {
            return Err(ProjektFehler::RevisionKonflikt {
                aktuell: alt.revision,
            });
        }
        if alt.archived {
            return Err(ProjektFehler::Archiviert);
        }
        let neu_klasse = a.data_class.unwrap_or(alt.data_class);
        if neu_klasse < alt.data_class {
            let hoechste: Option<i64> = sqlx::query_scalar(
                "SELECT MAX(data_class) FROM eintraege WHERE mandant = ? AND projekt_id = ?",
            )
            .bind(&m)
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if hoechste.is_some_and(|h| h > neu_klasse.zahl()) {
                return Err(ProjektFehler::HerabstufenGesperrt);
            }
        }
        let title = a.title.unwrap_or(alt.title);
        let goal = a.goal.unwrap_or(alt.goal);
        let description = a.description.unwrap_or(alt.description);
        let instructions = a.instructions.unwrap_or(alt.instructions);
        let retention = a.retention_days.unwrap_or(alt.retention_days);
        pruefe_felder(
            Some(&title),
            goal.as_deref(),
            description.as_deref(),
            instructions.as_deref(),
            retention,
        )?;
        let web = a.web_research_enabled.unwrap_or(alt.web_research_enabled);
        let n = sqlx::query(
            "UPDATE projekte SET title=?, goal=?, description=?, instructions=?, data_class=?,
             special_category=?, sovereignty=?, retention_days=?, web_research_enabled=?,
             revision = revision + 1, updated_ms=? WHERE mandant=? AND id=? AND revision=?",
        )
        .bind(&title)
        .bind(&goal)
        .bind(&description)
        .bind(&instructions)
        .bind(neu_klasse.zahl())
        .bind(a.special_category.unwrap_or(alt.special_category))
        .bind(a.sovereignty.unwrap_or(alt.sovereignty).text())
        .bind(retention)
        .bind(web)
        .bind(jetzt_ms())
        .bind(&m)
        .bind(id)
        .bind(erwartete_revision)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n != 1 {
            drop(tx);
            return Err(self.konflikt(&m, id).await);
        }
        tx.commit().await?;
        self.melden("projekt.geaendert", id, None, None);
        if neu_klasse < alt.data_class {
            self.melden("projekt.herabgestuft", id, None, None);
        }
        self.projekt_lesen(p, id).await
    }

    pub async fn archivieren(
        &self,
        p: &Principal,
        id: &str,
        erwartete_revision: i64,
        archiviert: bool,
    ) -> Ergebnis<Projekt> {
        self.revisionsschritt(p, id, erwartete_revision, "UPDATE projekte SET archived=?, revision = revision + 1, updated_ms=? WHERE mandant=? AND id=? AND revision=?", archiviert)
            .await?;
        self.melden(
            if archiviert {
                "projekt.archiviert"
            } else {
                "projekt.reaktiviert"
            },
            id,
            None,
            None,
        );
        self.projekt_lesen(p, id).await
    }

    /// Ordnet dem Projekt einen Plan zu (0..1 je Projekt, 0..1 Projekt je Plan, je Mandant).
    pub async fn plan_zuordnen(
        &self,
        p: &Principal,
        id: &str,
        erwartete_revision: i64,
        plan_ref: &str,
    ) -> Ergebnis<Projekt> {
        self.schreibbar()?;
        let plan = plan_pruefen(plan_ref)?;
        let m = mandant(p);
        let mut tx = self.schreib_tx(&m, Some(id)).await?;
        let alt = projekt_in_tx(&mut tx, &m, id).await?;
        if alt.revision != erwartete_revision {
            return Err(ProjektFehler::RevisionKonflikt {
                aktuell: alt.revision,
            });
        }
        if alt.archived {
            return Err(ProjektFehler::Archiviert);
        }
        if alt.plan_ref.is_some() {
            return Err(ProjektFehler::ProjektHatPlan);
        }
        plan_einfuegen(&mut tx, &m, id, &plan).await?;
        bump(&mut tx, &m, id, erwartete_revision).await?;
        tx.commit().await?;
        self.melden("projekt.plan.zugeordnet", id, None, None);
        self.projekt_lesen(p, id).await
    }

    pub async fn plan_loesen(
        &self,
        p: &Principal,
        id: &str,
        erwartete_revision: i64,
    ) -> Ergebnis<Projekt> {
        self.schreibbar()?;
        let m = mandant(p);
        let mut tx = self.schreib_tx(&m, Some(id)).await?;
        let alt = projekt_in_tx(&mut tx, &m, id).await?;
        if alt.revision != erwartete_revision {
            return Err(ProjektFehler::RevisionKonflikt {
                aktuell: alt.revision,
            });
        }
        if alt.archived {
            return Err(ProjektFehler::Archiviert);
        }
        sqlx::query("DELETE FROM plan_zuordnung WHERE mandant=? AND projekt_id=?")
            .bind(&m)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        bump(&mut tx, &m, id, erwartete_revision).await?;
        tx.commit().await?;
        self.melden("projekt.plan.geloest", id, None, None);
        self.projekt_lesen(p, id).await
    }

    /// Loescht das Projekt samt Eintraegen. Die Loeschmeldungen gehen in die Outbox; der Bericht listet
    /// Kopien ausserhalb des Projekts (ADR-0026 Abschnitt 8).
    pub async fn projekt_loeschen(&self, p: &Principal, id: &str) -> Ergebnis<LoeschBericht> {
        self.schreibbar()?;
        let m = mandant(p);
        let mut tx = self.schreib_tx(&m, None).await?;
        projekt_in_tx(&mut tx, &m, id).await?;
        let rows = sqlx::query(
            "SELECT id, ref, external_copies FROM eintraege WHERE mandant=? AND projekt_id=?",
        )
        .bind(&m)
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        let mut bericht = LoeschBericht::default();
        let jetzt = jetzt_ms();
        for r in &rows {
            let eid: String = r.get("id");
            let ref_: String = r.get("ref");
            let kopien: Vec<ExterneKopie> =
                serde_json::from_str(&r.get::<String, _>("external_copies")).unwrap_or_default();
            outbox(
                &mut tx,
                &m,
                id,
                Some(&eid),
                Some(&ref_),
                "projekt_geloescht",
                jetzt,
            )
            .await?;
            bericht
                .externe_kopien
                .extend(kopien.into_iter().map(|k| (eid.clone(), k)));
        }
        bericht.eintraege_entfernt = rows.len();
        outbox(&mut tx, &m, id, None, None, "projekt_geloescht", jetzt).await?;
        sqlx::query("DELETE FROM eintraege WHERE mandant=? AND projekt_id=?")
            .bind(&m)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM plan_zuordnung WHERE mandant=? AND projekt_id=?")
            .bind(&m)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM projekte WHERE mandant=? AND id=?")
            .bind(&m)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.melden("projekt.geloescht", id, None, None);
        Ok(bericht)
    }

    async fn revisionsschritt(
        &self,
        p: &Principal,
        id: &str,
        rev: i64,
        sql: &'static str,
        flag: bool,
    ) -> Ergebnis<()> {
        self.schreibbar()?;
        let m = mandant(p);
        let mut tx = self.schreib_tx(&m, Some(id)).await?;
        let alt = projekt_in_tx(&mut tx, &m, id).await?;
        if alt.revision != rev {
            return Err(ProjektFehler::RevisionKonflikt {
                aktuell: alt.revision,
            });
        }
        let n = sqlx::query(sql)
            .bind(flag)
            .bind(jetzt_ms())
            .bind(&m)
            .bind(id)
            .bind(rev)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if n != 1 {
            drop(tx);
            return Err(self.konflikt(&m, id).await);
        }
        tx.commit().await?;
        Ok(())
    }

    // ------------------------------------------------------------------ Eintraege

    /// Ordnet einen Verweis zu. Voraussetzungen: Leserecht in der Herkunft, Klasse <= Projektklasse,
    /// Art passt zur App im Verweis.
    pub async fn eintrag_zuordnen(
        &self,
        p: &Principal,
        projekt_id: &str,
        neu: NeuerEintrag,
        recht: &dyn Herkunftsrecht,
    ) -> Ergebnis<Projekteintrag> {
        self.schreibbar()?;
        eintrag_pruefen(&neu)?;
        let m = mandant(p);
        // Rechtepruefung ausserhalb der Sperre (Aufruf in die Herkunftsapp kann dauern).
        if !recht.darf_lesen(p, &neu.ref_) {
            return Err(ProjektFehler::KeinHerkunftsrecht);
        }
        let klasse = recht
            .klasse(p, &neu.ref_)
            .map_or(neu.data_class, |k| k.max(neu.data_class));
        // Projektpruefung und INSERT in einer Transaktion: Herabstufen kann nicht dazwischen treten.
        let mut tx = self.schreib_tx(&m, None).await?;
        let projekt = projekt_in_tx(&mut tx, &m, projekt_id).await?;
        if projekt.archived {
            return Err(ProjektFehler::Archiviert);
        }
        if klasse > projekt.data_class {
            return Err(ProjektFehler::KlasseUeberProjekt);
        }
        let id = neue_id("pje");
        let kopien = serde_json::to_string(&neu.external_copies).unwrap_or_else(|_| "[]".into());
        let r = sqlx::query(
            "INSERT INTO eintraege (id, mandant, projekt_id, kind, title, origin_app, ref, data_class,
             assigned_by, assigned_ms, preview, external_copies) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(&m)
        .bind(projekt_id)
        .bind(neu.kind.text())
        .bind(&neu.title)
        .bind(&neu.origin_app)
        .bind(&neu.ref_)
        .bind(klasse.zahl())
        .bind(neu.assigned_by.text())
        .bind(jetzt_ms())
        .bind(&neu.preview)
        .bind(&kopien)
        .execute(&mut *tx)
        .await;
        if let Err(e) = r {
            return Err(if ist_eindeutig(&e) {
                ProjektFehler::Doppelt
            } else {
                e.into()
            });
        }
        let row = sqlx::query(EINTRAG_SELECT_EINS)
            .bind(&m)
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
        tx.commit().await?;
        self.melden(
            "projekt.eintrag.zugeordnet",
            projekt_id,
            Some(&id),
            Some(neu.kind),
        );
        Ok(eintrag_aus(&row))
    }

    /// Listet Eintraege; Rechte werden bei jedem Aufruf erneut geprueft (Entzug wirkt sofort).
    pub async fn eintraege_auflisten(
        &self,
        p: &Principal,
        projekt_id: &str,
        recht: &dyn Herkunftsrecht,
    ) -> Ergebnis<Vec<Eintragsansicht>> {
        self.projekt_lesen(p, projekt_id).await?;
        let rows = sqlx::query(EINTRAG_SELECT_PROJEKT)
            .bind(mandant(p))
            .bind(projekt_id)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .iter()
            .map(eintrag_aus)
            .map(|e| {
                if recht.darf_lesen(p, &e.ref_) {
                    Eintragsansicht::Sichtbar(e)
                } else {
                    Eintragsansicht::NichtVerfuegbar {
                        id: e.id,
                        project_id: e.project_id,
                        kind: e.kind,
                    }
                }
            })
            .collect())
    }

    /// Entfernt einen Eintrag (nie die Herkunft) und stellt eine Loeschmeldung in die Outbox.
    pub async fn eintrag_entfernen(&self, p: &Principal, eintrag_id: &str) -> Ergebnis<()> {
        self.schreibbar()?;
        let m = mandant(p);
        let mut tx = self.schreib_tx(&m, None).await?;
        let row = sqlx::query(EINTRAG_SELECT_EINS)
            .bind(&m)
            .bind(eintrag_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ProjektFehler::NichtGefunden)?;
        let e = eintrag_aus(&row);
        entfernen_in_tx(&mut tx, &m, &e, "entfernt").await?;
        tx.commit().await?;
        self.melden(
            "projekt.eintrag.entfernt",
            &e.project_id,
            Some(&e.id),
            Some(e.kind),
        );
        Ok(())
    }

    // ------------------------------------------------------------------ Outbox und Abgleich

    pub async fn ausstehende_loeschungen(&self, p: &Principal) -> Ergebnis<Vec<Loeschmeldung>> {
        let rows = sqlx::query(
            "SELECT id, projekt_id, eintrag_id, ref, grund FROM loesch_outbox WHERE mandant=? ORDER BY id",
        )
        .bind(mandant(p))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| Loeschmeldung {
                id: r.get("id"),
                projekt_id: r.get("projekt_id"),
                eintrag_id: r.get("eintrag_id"),
                ref_: r.get("ref"),
                grund: r.get("grund"),
            })
            .collect())
    }

    /// Bestaetigt eine Loeschmeldung (der Verbraucher hat sie angewendet); danach ist sie weg.
    pub async fn bestaetigen(&self, p: &Principal, id: i64) -> Ergebnis<()> {
        self.schreibbar()?;
        let n = sqlx::query("DELETE FROM loesch_outbox WHERE mandant=? AND id=?")
            .bind(mandant(p))
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        if n == 0 {
            Err(ProjektFehler::NichtGefunden)
        } else {
            Ok(())
        }
    }

    /// Entfernt Eintraege, deren Ursprung nicht mehr existiert (je Mandant des `Principal`).
    pub async fn abgleich(
        &self,
        p: &Principal,
        bestand: &dyn Herkunftsbestand,
    ) -> Ergebnis<AbgleichBericht> {
        self.schreibbar()?;
        let m = mandant(p);
        let rows = sqlx::query(EINTRAG_SELECT_MANDANT)
            .bind(&m)
            .fetch_all(&self.pool)
            .await?;
        let mut bericht = AbgleichBericht {
            geprueft: rows.len(),
            ..Default::default()
        };
        for e in rows.iter().map(eintrag_aus) {
            match bestand.existiert(&m, &e.ref_) {
                Vorhanden::Ja => continue,
                Vorhanden::Unbekannt => {
                    bericht.unbekannt += 1;
                    continue;
                }
                Vorhanden::Nein => {}
            }
            let mut tx = self.schreib_tx(&m, None).await?;
            entfernen_in_tx(&mut tx, &m, &e, "waise").await?;
            tx.commit().await?;
            self.melden(
                "projekt.eintrag.entfernt",
                &e.project_id,
                Some(&e.id),
                Some(e.kind),
            );
            bericht.waisen_entfernt.push(e.id);
        }
        Ok(bericht)
    }
}

// ---------------------------------------------------------------------- Hilfsfunktionen

type Tx<'a> = sqlx::Transaction<'a, sqlx::Sqlite>;

const PROJEKT_SELECT_EINS: &str = "SELECT p.id, p.title, p.goal, p.description, p.instructions, p.data_class,
    p.special_category, p.sovereignty, p.retention_days, p.web_research_enabled, p.archived, p.revision,
    p.created_ms, p.updated_ms, z.plan_ref FROM projekte p
    LEFT JOIN plan_zuordnung z ON z.mandant = p.mandant AND z.projekt_id = p.id
    WHERE p.mandant = ? AND p.id = ?";
const PROJEKT_SELECT_ALLE: &str = "SELECT p.id, p.title, p.goal, p.description, p.instructions, p.data_class,
    p.special_category, p.sovereignty, p.retention_days, p.web_research_enabled, p.archived, p.revision,
    p.created_ms, p.updated_ms, z.plan_ref FROM projekte p
    LEFT JOIN plan_zuordnung z ON z.mandant = p.mandant AND z.projekt_id = p.id
    WHERE p.mandant = ? AND (? OR p.archived = 0) ORDER BY p.created_ms, p.rowid";
const EINTRAG_SELECT_EINS: &str =
    "SELECT id, projekt_id, kind, title, origin_app, ref, data_class, assigned_by,
    assigned_ms, preview, external_copies FROM eintraege WHERE mandant = ? AND id = ?";
const EINTRAG_SELECT_PROJEKT: &str = "SELECT id, projekt_id, kind, title, origin_app, ref, data_class, assigned_by,
    assigned_ms, preview, external_copies FROM eintraege WHERE mandant = ? AND projekt_id = ? ORDER BY assigned_ms, rowid";
const EINTRAG_SELECT_MANDANT: &str = "SELECT id, projekt_id, kind, title, origin_app, ref, data_class, assigned_by,
    assigned_ms, preview, external_copies FROM eintraege WHERE mandant = ? ORDER BY assigned_ms, rowid";

async fn vorhandene_version(pfad: &Path) -> Option<i64> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(pfad).read_only(true))
        .await
        .ok()?;
    let v: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _wai_migrations")
        .fetch_one(&pool)
        .await
        .ok()
        .flatten();
    pool.close().await;
    v
}

fn projekt_aus(r: &SqliteRow) -> Projekt {
    Projekt {
        schema_version: SCHEMA_VERSION.into(),
        id: r.get("id"),
        ts: zeitstempel(r.get("updated_ms")),
        trace_id: neue_trace_id(),
        title: r.get("title"),
        goal: r.get("goal"),
        description: r.get("description"),
        instructions: r.get("instructions"),
        data_class: Klasse::aus_zahl(r.get("data_class")),
        special_category: r.get("special_category"),
        sovereignty: Souveraenitaet::aus_text(&r.get::<String, _>("sovereignty")),
        retention_days: r.get("retention_days"),
        web_research_enabled: r.get("web_research_enabled"),
        archived: r.get("archived"),
        revision: r.get("revision"),
        plan_ref: r.get("plan_ref"),
        created_at: zeitstempel(r.get("created_ms")),
        updated_at: zeitstempel(r.get("updated_ms")),
    }
}

fn eintrag_aus(r: &SqliteRow) -> Projekteintrag {
    let ms: i64 = r.get("assigned_ms");
    Projekteintrag {
        schema_version: SCHEMA_VERSION.into(),
        id: r.get("id"),
        ts: zeitstempel(ms),
        trace_id: neue_trace_id(),
        project_id: r.get("projekt_id"),
        // Unbekannte Art kann nur aus einer neueren Version stammen; dann waere die DB nur lesend.
        kind: Art::aus_text(&r.get::<String, _>("kind")).unwrap_or(Art::Document),
        title: r.get("title"),
        origin_app: r.get("origin_app"),
        ref_: r.get("ref"),
        data_class: Klasse::aus_zahl(r.get("data_class")),
        assigned_by: ZugeordnetDurch::aus_text(&r.get::<String, _>("assigned_by")),
        assigned_at: zeitstempel(ms),
        preview: r.get("preview"),
        external_copies: serde_json::from_str(&r.get::<String, _>("external_copies"))
            .unwrap_or_default(),
    }
}

async fn projekt_in_tx(tx: &mut Tx<'_>, m: &str, id: &str) -> Ergebnis<Projekt> {
    let row = sqlx::query(PROJEKT_SELECT_EINS)
        .bind(m)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    row.map(|r| projekt_aus(&r))
        .ok_or(ProjektFehler::NichtGefunden)
}

async fn bump(tx: &mut Tx<'_>, m: &str, id: &str, rev: i64) -> Ergebnis<()> {
    let n = sqlx::query("UPDATE projekte SET revision = revision + 1, updated_ms = ? WHERE mandant=? AND id=? AND revision=?")
        .bind(jetzt_ms())
        .bind(m)
        .bind(id)
        .bind(rev)
        .execute(&mut **tx)
        .await?
        .rows_affected();
    if n == 1 {
        Ok(())
    } else {
        Err(ProjektFehler::RevisionKonflikt { aktuell: rev + 1 })
    }
}

async fn outbox(
    tx: &mut Tx<'_>,
    m: &str,
    projekt: &str,
    eintrag: Option<&str>,
    ref_: Option<&str>,
    grund: &str,
    ms: i64,
) -> Ergebnis<()> {
    sqlx::query("INSERT INTO loesch_outbox (mandant, projekt_id, eintrag_id, ref, grund, erzeugt_ms) VALUES (?,?,?,?,?,?)")
        .bind(m)
        .bind(projekt)
        .bind(eintrag)
        .bind(ref_)
        .bind(grund)
        .bind(ms)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn entfernen_in_tx(
    tx: &mut Tx<'_>,
    m: &str,
    e: &Projekteintrag,
    grund: &str,
) -> Ergebnis<()> {
    sqlx::query("DELETE FROM eintraege WHERE mandant=? AND id=?")
        .bind(m)
        .bind(&e.id)
        .execute(&mut **tx)
        .await?;
    outbox(
        tx,
        m,
        &e.project_id,
        Some(&e.id),
        Some(&e.ref_),
        grund,
        jetzt_ms(),
    )
    .await
}

async fn plan_einfuegen(tx: &mut Tx<'_>, m: &str, projekt: &str, plan: &str) -> Ergebnis<()> {
    let belegt: Option<String> =
        sqlx::query_scalar("SELECT projekt_id FROM plan_zuordnung WHERE mandant=? AND plan_ref=?")
            .bind(m)
            .bind(plan)
            .fetch_optional(&mut **tx)
            .await?;
    if belegt.is_some() {
        return Err(ProjektFehler::PlanVergeben);
    }
    let r =
        sqlx::query("INSERT INTO plan_zuordnung (mandant, projekt_id, plan_ref) VALUES (?,?,?)")
            .bind(m)
            .bind(projekt)
            .bind(plan)
            .execute(&mut **tx)
            .await;
    match r {
        Ok(_) => Ok(()),
        Err(e) if ist_eindeutig(&e) => Err(if e.to_string().contains("plan_ref") {
            ProjektFehler::PlanVergeben
        } else {
            ProjektFehler::ProjektHatPlan
        }),
        Err(e) => Err(e.into()),
    }
}

fn ist_eindeutig(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d) if d.is_unique_violation())
}

fn plan_pruefen(r: &str) -> Ergebnis<String> {
    let l = wai_links::parse(r).map_err(|e| ProjektFehler::LinkUngueltig(e.code().into()))?;
    if l.app != "planner" || l.art != "plan" {
        return Err(ProjektFehler::ArtPasstNichtZurApp);
    }
    if !wai_muster_ok(r) {
        return Err(ProjektFehler::LinkUngueltig("link.muster".into()));
    }
    Ok(l.to_string())
}

fn ist_belegt(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d)
        if matches!(d.code().as_deref(), Some("5" | "6" | "261" | "262" | "517"))
            || d.message().contains("locked") || d.message().contains("busy"))
}

fn laenge(feld: &str, s: &str, min: usize, max: usize) -> Ergebnis<()> {
    let n = s.chars().count();
    if n < min || n > max {
        return Err(ProjektFehler::Ungueltig(format!(
            "{feld}: Laenge {min}..={max} erwartet"
        )));
    }
    Ok(())
}

fn pruefe_felder(
    title: Option<&str>,
    goal: Option<&str>,
    description: Option<&str>,
    instructions: Option<&str>,
    retention: Option<i64>,
) -> Ergebnis<()> {
    if let Some(t) = title {
        laenge("title", t, 1, 300)?;
    }
    if let Some(g) = goal {
        laenge("goal", g, 0, 2000)?;
    }
    if let Some(d) = description {
        laenge("description", d, 0, 8000)?;
    }
    if let Some(i) = instructions {
        laenge("instructions", i, 0, 8000)?;
    }
    if retention.is_some_and(|r| r < 1) {
        return Err(ProjektFehler::Ungueltig(
            "retention_days: mindestens 1".into(),
        ));
    }
    Ok(())
}

fn eintrag_pruefen(e: &NeuerEintrag) -> Ergebnis<()> {
    laenge("title", &e.title, 1, 300)?;
    if let Some(v) = &e.preview {
        laenge("preview", v, 0, 280)?;
    }
    if e.external_copies.len() > 100 {
        return Err(ProjektFehler::Ungueltig(
            "external_copies: hoechstens 100".into(),
        ));
    }
    if !slug_ok(&e.origin_app) {
        return Err(ProjektFehler::Ungueltig(
            "origin_app: kein gueltiger Slug".into(),
        ));
    }
    let app = match e.kind.erwartete_app() {
        Some(erwartet) => {
            let l = wai_links::parse(&e.ref_)
                .map_err(|x| ProjektFehler::LinkUngueltig(x.code().into()))?;
            if l.app != erwartet {
                return Err(ProjektFehler::ArtPasstNichtZurApp);
            }
            l.app
        }
        None => {
            // Andere Arten: gueltiger wai://-Link im Sinne des Vertragsmusters (ADR-0018-Allow-List
            // kennt nicht jede Herkunftsart).
            if !wai_muster_ok(&e.ref_) {
                return Err(ProjektFehler::LinkUngueltig("link.muster".into()));
            }
            e.ref_["wai://".len()..]
                .split('/')
                .next()
                .unwrap_or_default()
                .to_string()
        }
    };
    if app != e.origin_app {
        return Err(ProjektFehler::ArtPasstNichtZurApp);
    }
    Ok(())
}
