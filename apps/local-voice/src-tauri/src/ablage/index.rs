//! Lokaler Abrufindex der Ablage (Spec §4.3, ADR-0029 E2): `ablage/index.db`, abgeleitet, löschbar, nie
//! exportiert oder synchronisiert.
//!
//! * Textstellen und Ranking (FTS5/BM25) liegen im vendorten `wai-retrieval` (Mandant `ablage`, Quelle =
//!   `dat_…`). Jede Textstelle ist eine „Seite“ des Dokuments, damit Stücke und Anker hier bestimmt
//!   werden: Stücke von höchstens [`STUECK`] Zeichen je Abschnitt mit [`UEBERLAPPUNG`] Zeichen Überlappung.
//! * Begleittabellen `ablage_dok` (Felder je Datei: Fassung, `textrev`, Ordnerkette, Klasse,
//!   `nur_geraet`, Papierkorb, Stand, Stempel) und `ablage_stueck` (Seite → Abschnitt, Anker, Pfad).
//! * Filter vor dem Ranking: das Quellrecht von `wai-retrieval` wird in derselben SQL-Abfrage geprüft.
//!   Lebende Dateien tragen den Leser [`LESER_SUCHE`], Dateien im Papierkorb keinen; sie fallen damit vor
//!   dem Ranking heraus. D3 ergänzt weitere Leser (Freigabe, Klasse).
//! * [`abgleichen`] ist der einzige Schreibweg: vergleicht den Stempel je Datei mit dem Stand der Ablage
//!   und schreibt nur Abweichungen (Ablegen, Löschen, Verschieben, Papierkorb, Wiederherstellen, neue
//!   Textfassung). Zweimal hintereinander ändert nichts; [`neu_aufbauen`] löscht `index.db` und gleicht ab.
//! * D6: Abgleich und Suche nehmen eine [`AblageEinbettung`] (Standard: die wirksame des Prozesses).
//!   Ohne Vektoren bleibt alles lexikalisch wie in D2; mit Vektoren mischt `wai-retrieval` beide Listen
//!   per RRF. [`vektoren_nachziehen`] bettet nach dem Einschalten alle Stücke ein.

use super::extrakt::Textfassung;
use super::semantik::AblageEinbettung;
use super::speicher::{Ablage, IndexQuelle};
use super::{datei_id_pruefen, Fehler, Res};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use wai_retrieval::{Acl, IngestDoc, RetrievalError, Retriever};

pub const INDEX: &str = "index.db";
pub const MANDANT: &str = "ablage";
/// Leser der Ablage-Suche (Oberfläche, ohne KI).
pub const LESER_SUCHE: &str = "ablage-suche";
pub const STUECK: usize = 220;
pub const UEBERLAPPUNG: usize = 40;
const KANDIDATEN: usize = 60;
const STELLEN_JE_DATEI: usize = 3;

static SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn sperren() -> std::sync::MutexGuard<'static, ()> {
    SPERRE.lock().unwrap_or_else(|e| e.into_inner())
}

fn fehler() -> Fehler {
    Fehler::neu("ablage.index", "Der Suchindex ist gerade nicht verfügbar.")
}

/// Fundstelle in einer Datei.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct AblageFundstelle {
    /// Anker nach Markenformat v1 (`seite=3`, `absatz=12`, `blatt=…&zeilen=2-21`, `teil=kopf`, …).
    pub anker: String,
    /// Überschriftenpfad (DOCX/ODT/HTML) bzw. Blattname.
    pub pfad: Vec<String>,
    pub auszug: String,
}

/// Suchtreffer: Name und/oder Inhalt einer lebenden Datei.
#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
pub struct AblageSuchtreffer {
    pub dat: String,
    pub name: String,
    pub ordner: String,
    pub typ: String,
    /// Bester Rang der Fundstellen (0..1).
    pub rang: f64,
    pub stellen: Vec<AblageFundstelle>,
}

/// Stücke eines Abschnitts: höchstens `max` Zeichen, Überlappung `ueber` Zeichen, Schnitt möglichst an
/// Leerraum. Offsets in Zeichen.
pub fn stuecke(text: &str, max: usize, ueber: usize) -> Vec<String> {
    let z: Vec<char> = text.chars().collect();
    let mut aus = Vec::new();
    let mut start = 0usize;
    while start < z.len() {
        while start < z.len() && z[start].is_whitespace() {
            start += 1;
        }
        if start >= z.len() {
            break;
        }
        let mut ende = (start + max).min(z.len());
        if ende < z.len() {
            if let Some(i) = (start + max / 2..ende)
                .rev()
                .find(|&i| z[i].is_whitespace())
            {
                ende = i;
            }
        }
        aus.push(z[start..ende].iter().collect::<String>().trim().to_string());
        if ende >= z.len() {
            break;
        }
        let mut naechster = ende.saturating_sub(ueber).max(start + 1);
        // Am Wortanfang beginnen.
        while naechster < ende && !z[naechster - 1].is_whitespace() {
            naechster += 1;
        }
        start = naechster;
    }
    aus.retain(|s| !s.is_empty());
    aus
}

fn stempel(q: &IndexQuelle) -> String {
    let mut h = Sha256::new();
    for teil in [
        q.fas.as_str(),
        q.textrev.as_deref().unwrap_or("-"),
        &q.ordnerkette.join("/"),
        &q.klasse,
        if q.nur_geraet { "1" } else { "0" },
        if q.papierkorb { "1" } else { "0" },
        &q.stand,
        "v1",
    ] {
        h.update(teil.as_bytes());
        h.update([0u8]);
    }
    super::extrakt::hex(&h.finalize())
}

fn verbinden(pfad: &Path) -> Res<Connection> {
    let c = Connection::open(pfad).map_err(|_| fehler())?;
    c.busy_timeout(std::time::Duration::from_secs(10))
        .map_err(|_| fehler())?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS ablage_dok(
            dat TEXT PRIMARY KEY, stempel TEXT NOT NULL, version INTEGER NOT NULL,
            fas TEXT NOT NULL, textrev TEXT, ordnerkette TEXT NOT NULL, klasse TEXT NOT NULL,
            nur_geraet INTEGER NOT NULL, papierkorb INTEGER NOT NULL, stand TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS ablage_stueck(
            dat TEXT NOT NULL, seite INTEGER NOT NULL, abschnitt INTEGER NOT NULL,
            anker TEXT NOT NULL, pfad TEXT NOT NULL, PRIMARY KEY(dat, seite));",
    )
    .map_err(|_| fehler())?;
    Ok(c)
}

fn laufzeit() -> Res<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| fehler())
}

fn index_pfad(a: &Ablage) -> Res<PathBuf> {
    // `ablage/` selbst darf keine Verknüpfung sein; `index.db` auch nicht.
    if let Ok(m) = std::fs::symlink_metadata(a.dir()) {
        if super::ist_umweg(&m) {
            return Err(fehler());
        }
    }
    let p = a.dir().join(INDEX);
    if let Ok(m) = std::fs::symlink_metadata(&p) {
        if super::ist_umweg(&m) {
            return Err(fehler());
        }
    }
    Ok(p)
}

/// Textfassung lesen und prüfen (passende `textrev`); `None` bei fehlender oder fremder Datei.
fn textfassung_lesen(a: &Ablage, dat: &str, textrev: &str) -> Option<Textfassung> {
    let p = a.text_pfad(dat, textrev);
    let m = std::fs::symlink_metadata(&p).ok()?;
    if super::ist_umweg(&m) || !m.is_file() {
        return None;
    }
    let t: Textfassung = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
    (t.textrev == textrev).then_some(t)
}

/// Bericht eines Abgleichs (Tests, Oberfläche).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageIndexBericht {
    pub geschrieben: u32,
    pub entfernt: u32,
    pub dateien: u32,
    pub stuecke: u32,
}

/// Bringt `index.db` auf den Stand der Ablage (nur Abweichungen werden geschrieben), mit der wirksamen
/// Einbettung des Prozesses ([`super::semantik::aktuell`]).
pub fn abgleichen(a: &Ablage) -> Res<AblageIndexBericht> {
    abgleichen_mit(a, &super::semantik::aktuell())
}

/// Wie [`abgleichen`] mit ausdrücklicher Einbettung. Neue Stücke bekommen sofort Vektoren, wenn der
/// Index schon im Vektorraum von `emb` liegt; sonst bleiben sie lexikalisch bis [`vektoren_nachziehen`].
pub fn abgleichen_mit(a: &Ablage, emb: &AblageEinbettung) -> Res<AblageIndexBericht> {
    let _g = sperren();
    let quellen = a.index_quellen()?;
    let pfad = index_pfad(a)?;
    std::fs::create_dir_all(a.dir()).map_err(|_| fehler())?;
    let mut c = verbinden(&pfad)?;
    let mut bekannt: BTreeMap<String, (String, i64)> = BTreeMap::new();
    {
        let mut q = c
            .prepare("SELECT dat, stempel, version FROM ablage_dok")
            .map_err(|_| fehler())?;
        let zeilen = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|_| fehler())?;
        for z in zeilen {
            let (d, s, v): (String, String, i64) = z.map_err(|_| fehler())?;
            bekannt.insert(d, (s, v));
        }
    }
    let soll: BTreeSet<&str> = quellen.iter().map(|q| q.dat.as_str()).collect();
    // Verwaist: Dokumente im Retriever ohne lebenden Eintrag (z. B. nach einem Abbruch zwischen beiden
    // Schreibvorgängen) werden wie gelöschte Dateien entfernt.
    let mut waisen: BTreeSet<String> = BTreeSet::new();
    if let Ok(mut q) = c.prepare("SELECT source_id FROM docs WHERE tenant = ?1") {
        if let Ok(zeilen) = q.query_map(params![MANDANT], |r| r.get::<_, String>(0)) {
            waisen.extend(zeilen.flatten().filter(|d| !soll.contains(d.as_str())));
        }
    }
    let weg: Vec<String> = bekannt
        .keys()
        .filter(|d| !soll.contains(d.as_str()))
        .cloned()
        .chain(waisen.into_iter().filter(|d| !bekannt.contains_key(d)))
        .collect();
    let neu: Vec<(&IndexQuelle, String, i64)> = quellen
        .iter()
        .filter_map(|q| {
            let s = stempel(q);
            match bekannt.get(&q.dat) {
                Some((alt, _)) if *alt == s => None,
                Some((_, v)) => Some((q, s, v + 1)),
                None => Some((q, s, 1)),
            }
        })
        .collect();
    let mut bericht = AblageIndexBericht {
        dateien: quellen.len() as u32,
        ..Default::default()
    };
    if !weg.is_empty() || !neu.is_empty() {
        let rt = laufzeit()?;
        rt.block_on(async {
            let r = Retriever::open(&pfad, emb.clone())
                .await
                .map_err(|_| fehler())?;
            for d in &weg {
                r.delete(d).await.map_err(|_| fehler())?;
                let tx = c.transaction().map_err(|_| fehler())?;
                tx.execute("DELETE FROM ablage_stueck WHERE dat = ?1", params![d])
                    .map_err(|_| fehler())?;
                tx.execute("DELETE FROM ablage_dok WHERE dat = ?1", params![d])
                    .map_err(|_| fehler())?;
                tx.commit().map_err(|_| fehler())?;
                bericht.entfernt += 1;
            }
            for (q, s, version) in &neu {
                let text = q
                    .textrev
                    .as_deref()
                    .and_then(|rev| textfassung_lesen(a, &q.dat, rev));
                let mut seiten: Vec<String> = Vec::new();
                let mut zuordnung: Vec<(usize, String, String)> = Vec::new();
                if let Some(t) = &text {
                    for (i, ab) in t.abschnitte.iter().enumerate() {
                        let pfad = serde_json::to_string(&ab.pfad).unwrap_or_else(|_| "[]".into());
                        for st in stuecke(&ab.text, STUECK, UEBERLAPPUNG) {
                            seiten.push(st);
                            zuordnung.push((i, ab.anker.clone(), pfad.clone()));
                        }
                    }
                }
                if seiten.is_empty() {
                    r.delete(&q.dat).await.map_err(|_| fehler())?;
                } else {
                    let leser = if q.papierkorb {
                        vec![]
                    } else {
                        vec![LESER_SUCHE.to_string()]
                    };
                    r.ingest(&IngestDoc {
                        source_id: q.dat.clone(),
                        tenant: MANDANT.into(),
                        version: (*version).clamp(1, u32::MAX as i64) as u32,
                        pages: seiten.clone(),
                        acl: Acl::Principals(leser),
                    })
                    .await
                    .map_err(|_| fehler())?;
                }
                let tx = c.transaction().map_err(|_| fehler())?;
                tx.execute("DELETE FROM ablage_stueck WHERE dat = ?1", params![q.dat])
                    .map_err(|_| fehler())?;
                for (seite, (abschnitt, anker, pfad)) in zuordnung.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO ablage_stueck(dat, seite, abschnitt, anker, pfad) VALUES(?1,?2,?3,?4,?5)",
                        params![q.dat, seite as i64 + 1, *abschnitt as i64, anker, pfad],
                    )
                    .map_err(|_| fehler())?;
                }
                tx.execute(
                    "INSERT OR REPLACE INTO ablage_dok(dat, stempel, version, fas, textrev, ordnerkette,
                        klasse, nur_geraet, papierkorb, stand) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        q.dat,
                        s,
                        version,
                        q.fas,
                        q.textrev,
                        q.ordnerkette.join("/"),
                        q.klasse,
                        q.nur_geraet,
                        q.papierkorb,
                        q.stand
                    ],
                )
                .map_err(|_| fehler())?;
                tx.commit().map_err(|_| fehler())?;
                bericht.geschrieben += 1;
            }
            Ok::<_, Fehler>(())
        })?;
    }
    // Stücke ohne Dokumenteintrag (verwaist) fallen weg.
    c.execute(
        "DELETE FROM ablage_stueck WHERE dat NOT IN (SELECT dat FROM ablage_dok)",
        [],
    )
    .map_err(|_| fehler())?;
    bericht.stuecke = c
        .query_row("SELECT COUNT(*) FROM ablage_stueck", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|_| fehler())? as u32;
    Ok(bericht)
}

/// „Index neu aufbauen“: `index.db` löschen und vollständig abgleichen (Extraktor-/Embedder-Wechsel,
/// beschädigter Index).
pub fn neu_aufbauen(a: &Ablage) -> Res<AblageIndexBericht> {
    neu_aufbauen_mit(a, &super::semantik::aktuell())
}

pub fn neu_aufbauen_mit(a: &Ablage, emb: &AblageEinbettung) -> Res<AblageIndexBericht> {
    {
        let _g = sperren();
        let p = index_pfad(a)?;
        for endung in ["", "-wal", "-shm", "-journal"] {
            let q = PathBuf::from(format!("{}{endung}", p.display()));
            match std::fs::remove_file(&q) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(fehler()),
            }
        }
    }
    abgleichen_mit(a, emb)
}

/// Bettet alle Stücke neu ein, wenn der Index nicht im Vektorraum von `emb` liegt oder Stücke ohne
/// Vektor hat (Einschalten, Modellwechsel, Stücke aus der lexikalischen Zeit). Hält die Indexsperre
/// nicht: `wai-retrieval` bettet außerhalb der Schreibtransaktion ein und verwirft das Ergebnis, wenn
/// parallel neu indexiert wurde (dann ein zweiter Versuch). Liefert die Zahl eingebetteter Stücke.
pub fn vektoren_nachziehen(a: &Ablage, emb: &AblageEinbettung) -> Res<u32> {
    if !emb.ist_semantisch() {
        return Ok(0);
    }
    let pfad = index_pfad(a)?;
    if !pfad.is_file() {
        return Ok(0);
    }
    laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, emb.clone())
            .await
            .map_err(|_| fehler())?;
        for _ in 0..2 {
            let st = r.status_for(MANDANT).await.map_err(|_| fehler())?;
            if !st.reindex_needed && st.missing_vectors == 0 {
                return Ok(0);
            }
            match r.reindex(MANDANT).await {
                Ok(n) => return Ok(n.min(u32::MAX as usize) as u32),
                Err(RetrievalError::Conflict(_)) => continue,
                Err(_) => return Err(fehler()),
            }
        }
        Err(fehler())
    })
}

/// Zustand der Vektoren für die Oberfläche: (Neuaufbau nötig, Stücke ohne Vektor). Ohne Index: (false, 0).
pub fn vektor_stand(a: &Ablage, emb: &AblageEinbettung) -> Res<(bool, u32)> {
    if !emb.ist_semantisch() {
        return Ok((false, 0));
    }
    let pfad = index_pfad(a)?;
    if !pfad.is_file() {
        return Ok((false, 0));
    }
    laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, emb.clone())
            .await
            .map_err(|_| fehler())?;
        let st = r.status_for(MANDANT).await.map_err(|_| fehler())?;
        Ok((
            st.reindex_needed,
            st.missing_vectors.clamp(0, u32::MAX as i64) as u32,
        ))
    })
}

/// Leert den Abfrage-Zwischenspeicher von `wai-retrieval` (Moduswechsel: eine zwischengespeicherte
/// Rangfolge der anderen Suchart darf nicht wiederkommen).
pub fn zwischenspeicher_leeren(a: &Ablage) -> Res<()> {
    let _g = sperren();
    let pfad = index_pfad(a)?;
    if !pfad.is_file() {
        return Ok(());
    }
    let c = verbinden(&pfad)?;
    let _ = c.execute_batch("DELETE FROM qcache; DELETE FROM qcache_src;");
    Ok(())
}

fn leser() -> wai_security::Principal {
    use wai_security::{
        resolve_principal, ActorId, ClientClaims, DataClass, Role, ServerSession, TenantId,
    };
    let sitzung = ServerSession {
        tenant: TenantId(MANDANT.into()),
        actor: ActorId(LESER_SUCHE.into()),
        role: Role::Viewer,
        max_class: DataClass::Restricted,
        tool_rights: Default::default(),
    };
    resolve_principal(&sitzung, &ClientClaims::default()).expect("fester, gültiger Mandant")
}

/// Volltextsuche (ohne KI): Fundstellen lebender Dateien mit Auszug und Anker, beste zuerst. Der
/// Papierkorb ist schon in SQL ausgeschlossen; zur Sicherheit wird gegen den aktuellen Stand gefiltert.
pub fn suchen(a: &Ablage, anfrage: &str) -> Res<Vec<AblageSuchtreffer>> {
    suchen_mit(a, anfrage, &super::semantik::aktuell())
}

/// Wie [`suchen`] mit ausdrücklicher Einbettung: mit Vektoren hybrid (BM25 + Kosinus, RRF), sonst
/// lexikalisch. Die Vektorliste wirkt nur, wenn der Index im Vektorraum von `emb` liegt.
pub fn suchen_mit(
    a: &Ablage,
    anfrage: &str,
    emb: &AblageEinbettung,
) -> Res<Vec<AblageSuchtreffer>> {
    let anfrage: String = anfrage.chars().take(200).collect();
    if wai_retrieval::content_tokens(&anfrage).is_empty() {
        return Ok(vec![]);
    }
    let pfad = index_pfad(a)?;
    if !pfad.is_file() {
        return Ok(vec![]);
    }
    let _g = sperren();
    let trace = format!("{:032x}", ulid::Ulid::new().0);
    let antwort = laufzeit()?.block_on(async {
        let r = Retriever::open(&pfad, emb.clone())
            .await
            .map_err(|_| fehler())?;
        r.query(&leser(), &trace, &anfrage, KANDIDATEN)
            .await
            .map_err(|_| fehler())
    })?;
    let c = verbinden(&pfad)?;
    let dats: Vec<String> = antwort
        .citations
        .iter()
        .map(|z| z.artifact_id.clone())
        .filter(|d| datei_id_pruefen(d).is_ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let infos = a.treffer_infos(&dats)?;
    let mut je_datei: BTreeMap<String, AblageSuchtreffer> = BTreeMap::new();
    let mut reihenfolge: Vec<String> = Vec::new();
    for z in &antwort.citations {
        let Some(info) = infos.get(&z.artifact_id) else {
            continue;
        };
        let seite = z.locator.page.unwrap_or(0) as i64;
        let zeile: Option<(String, String, bool)> = c
            .query_row(
                "SELECT s.anker, s.pfad, d.papierkorb FROM ablage_stueck s
                 JOIN ablage_dok d ON d.dat = s.dat WHERE s.dat = ?1 AND s.seite = ?2",
                params![z.artifact_id, seite],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(|_| fehler())?;
        let Some((anker, pfad, im_korb)) = zeile else {
            continue;
        };
        if im_korb {
            continue;
        }
        let t = je_datei.entry(z.artifact_id.clone()).or_insert_with(|| {
            reihenfolge.push(z.artifact_id.clone());
            AblageSuchtreffer {
                dat: z.artifact_id.clone(),
                name: info.name.clone(),
                ordner: info.ordner.clone(),
                typ: info.typ.clone(),
                rang: z.score,
                stellen: vec![],
            }
        });
        if t.stellen.len() >= STELLEN_JE_DATEI || t.stellen.iter().any(|s| s.anker == anker) {
            continue;
        }
        t.stellen.push(AblageFundstelle {
            anker,
            pfad: serde_json::from_str(&pfad).unwrap_or_default(),
            auszug: z.quote.clone(),
        });
    }
    Ok(reihenfolge
        .into_iter()
        .filter_map(|d| je_datei.remove(&d))
        .collect())
}
