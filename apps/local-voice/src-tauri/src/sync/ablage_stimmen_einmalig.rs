//! Eng begrenzter Transport des Examples: nur Speicher, kein Ledger, kein Pull in die lokale Ablage.
use super::*;
use crate::ablage::system;
use super::super::client::RemoteObject;

type Res<T> = Result<T, &'static str>;

fn kandidat(collection: &'static str, id: &str, wert: Value, refs: Vec<String>) -> Res<Kandidat> {
    let plain = collect::canonical(&wert);
    if plain.len() > collect::MAX_PLAINTEXT { return Err("Stimmobjekt überschreitet das Sync-Limit."); }
    Ok(Kandidat { collection, id: id.into(), art: Art::Stand, hash: collect::hash(&wert), plain, refs })
}

fn aktuell_oeffnen(konto: &Konto<'_>, remote: &RemoteObject) -> Res<Eingang> {
    let key_id = crypto::key_id(konto.key);
    if remote.deleted || remote.key_id != key_id || remote.revision <= 0 {
        return Err("Systemobjekt gelöscht oder anderer Schlüssel; kein sicherer Parallelbetrieb für diesen Stand.");
    }
    oeffnen(konto, &key_id, &remote.collection, &remote.object_id,
        remote.blob_refs.as_deref().unwrap_or(&[]), &remote.payload)
        .map(|(e, _)| e).ok_or("Systemobjekt nicht verifizierbar; Upload abgebrochen.")
}

fn datei_ergaenzen(konto: &Konto<'_>, geplant: &Datei, bytes: &[u8], remote: &Datei) -> Res<Datei> {
    if !crate::managers::tts::phonon_voices::sync_datei_gueltig(remote, konto.key, Some(bytes))
        || remote.id != geplant.id || remote.ordner.wert != geplant.ordner.wert
        || remote.extra["phonon_voice"][1]["uid"] != geplant.extra["phonon_voice"][1]["uid"]
        || remote.extra["phonon_voice"][1]["sha256"] != blobs::sha256_hex(bytes)
        || remote.nur_geraet_min || remote.klasse_min.wert != "internal" {
        return Err("Hub-Stimme weicht vom Master ab; App-Abgleich zuerst abschließen.");
    }
    // Fassungen, Name und sämtliche unbekannten Felder des Hub-Stands bleiben erhalten.
    let mut d = remote.clone();
    crate::managers::tts::phonon_voices::emb_uebernehmen(&mut d, geplant)
        .map_err(|_| "emb-Metadaten nicht übernehmbar.")?;
    zf::phonon_emb_begrenzen(&mut d, collect::MAX_PLAINTEXT);
    for field in geplant.extra.keys().filter(|k| k.starts_with("phonon_emb_")) {
        if d.extra.get(field).is_none_or(|v| v[1] != geplant.extra[field][1]) {
            return Err("emb-Fassungen passen nicht vollständig in das Hub-Objekt.");
        }
    }
    Ok(d)
}

async fn push<H: AblageHub + ?Sized>(hub: &H, konto: &Konto<'_>, k: &Kandidat, base: i64) -> Res<()> {
    let obj = versiegeln(konto, &crypto::key_id(konto.key), k, base).map_err(|_| "Objektverschlüsselung fehlgeschlagen.")?;
    let ergebnisse = hub.push(konto.device_id, &[obj]).await.map_err(|_| "Hub-Push fehlgeschlagen; Bestätigung unbekannt.")?;
    if ergebnisse.len() != 1 || ergebnisse[0].collection != k.collection || ergebnisse[0].object_id != k.id
        || ergebnisse[0].outcome != "accepted" || ergebnisse[0].revision <= base {
        return Err("Hub-Push nicht bestätigt (mögliche parallele Änderung); kein Erzwingen oder Wiederholen.");
    }
    Ok(())
}

pub(crate) async fn hochladen<H: AblageHub + ?Sized>(hub: &H, konto: &Konto<'_>,
    stimmen: Vec<(Ordner, Datei, Vec<u8>)>, aktuell: impl Fn() -> bool) -> Result<usize, (usize, &'static str)> {
    let mut bestaetigt = 0;
    let result: Res<()> = async {
        if !aktuell() { return Err("Lokales Konto oder Stimmen geändert; kein sicherer Parallelbetrieb für diesen Stand."); }
        let mut gesucht = BTreeSet::new();
        for (o, d, bytes) in &stimmen {
            if !system::ist_ordner(&o.id) || !system::ist_datei(&d.id) || d.ordner.wert != o.id
                || !crate::managers::tts::phonon_voices::sync_datei_gueltig(d, konto.key, Some(bytes)) {
                return Err("Upload außerhalb des Stimm-Systemordners abgelehnt.");
            }
            gesucht.insert((SAMMLUNG_ORDNER.to_string(), o.id.clone()));
            gesucht.insert((SAMMLUNG_DATEI.to_string(), d.id.clone()));
        }
        if gesucht.is_empty() { return Ok(()); }
        // Nur die benötigten Systemobjekte merken. Keine fremden Objekte lokal anwenden.
        let mut remote = BTreeMap::new();
        for coll in [SAMMLUNG_ORDNER, SAMMLUNG_DATEI] {
            let mut cursor = 0;
            loop {
                let seite = hub.pull_sammlung(cursor, 200, coll).await.map_err(|_| "Hub-Stand nicht vollständig lesbar.")?;
                for obj in seite.objects {
                    let k = (obj.collection.clone(), obj.object_id.clone());
                    if obj.collection != coll { return Err("Unerwartete Hub-Sammlung."); }
                    if gesucht.contains(&k) { remote.insert(k, obj); }
                }
                if !seite.more { break; }
                if seite.cursor <= cursor { return Err("Hub-Cursor ohne Fortschritt."); }
                cursor = seite.cursor;
            }
        }
        let mut plan = Vec::new();
        for (o, d, bytes) in stimmen {
            let mut o_neu = o.clone();
            let mut d_neu = d.clone();
            let mut o_base = 0;
            let mut d_base = 0;
            if let Some(obj) = remote.get(&(SAMMLUNG_ORDNER.into(), o.id.clone())) {
                let Eingang::Ordner(alt) = aktuell_oeffnen(konto, obj)? else { return Err("Kein Systemordner."); };
                if alt.eltern.wert.is_some() || alt.papierkorb.wert.is_some() || alt.nur_geraet.wert
                    || alt.datenklasse.wert != "internal" || alt.name.wert != ".lv-stimmen" {
                    return Err("Systemordner hat abweichende Teilnahme; App-Abgleich erforderlich.");
                }
                o_neu = alt;
                o_base = obj.revision;
            }
            if let Some(obj) = remote.get(&(SAMMLUNG_DATEI.into(), d.id.clone())) {
                let Eingang::Datei(alt) = aktuell_oeffnen(konto, obj)? else { return Err("Keine Systemstimme."); };
                d_neu = datei_ergaenzen(konto, &d, &bytes, &alt)?;
                d_base = obj.revision;
            }
            plan.push((kandidat(SAMMLUNG_ORDNER, &o_neu.id, wert_ordner(&o_neu), vec![])?, o_base,
                kandidat(SAMMLUNG_DATEI, &d_neu.id, wert_datei(&d_neu), hub_refs(konto.key, &d_neu))?, d_base, bytes));
        }
        let mut ordner = BTreeSet::new();
        let mut budget = u64::MAX;
        for (o, o_base, d, d_base, bytes) in plan {
            if !aktuell() { return Err("Lokales Konto oder Stimmen geändert; Upload angehalten."); }
            if ordner.insert(o.id.clone()) { push(hub, konto, &o, o_base).await?; }
            // Der bestehende Blob-Transport übernimmt Verschlüsselung, Stücke und Wiederaufnahme.
            let hid = hub_blob(konto.key, &blobs::sha256_hex(&bytes)).ok_or("Ungültiger Master-Hash.")?;
            hub.reservieren(&hid, konto.device_id).await.map_err(|_| "Blob-Reservierung fehlgeschlagen.")?;
            let size = bytes.len() as u64;
            let r = blobs::upload(hub, konto.key, konto.user_id, &hid, &Source::Bytes(bytes), size, None, &mut budget)
                .await.map_err(|_| "Blob-Upload fehlgeschlagen.")?;
            if r != Upload::Done { return Err("Blob-Upload unvollständig."); }
            if !aktuell() { return Err("Lokales Konto oder Stimmen geändert; Objekt nicht gesendet."); }
            push(hub, konto, &d, d_base).await?;
            bestaetigt += 1;
        }
        Ok(())
    }.await;
    result.map(|()| bestaetigt).map_err(|text| (bestaetigt, text))
}
