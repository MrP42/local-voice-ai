//! Bewertung ausschliesslich aus gespeicherten Antworten und fluechtigem Cache.
use super::*;
use chrono::TimeZone;
use crate::managers::integrations::m365::polls::Availability;

/// Unbekannte/mehrdeutige Zonen und DST-Grenzen werden nicht geraten.
pub(super) fn intervals(window: &ScheduleWindow) -> Option<Vec<(i64,i64)>> {
    let zone: calcard::common::timezone::Tz = window.time_zone.parse().ok()?;
    let zone = zone.to_resolved()?;
    let start = chrono::NaiveDateTime::parse_from_str(&window.start,"%Y-%m-%dT%H:%M:%S").ok()?;
    (0..window.intervals().ok()?).map(|n| {
        let a = zone.from_local_datetime(&(start + chrono::Duration::minutes(n as i64 * 30))).single()?.timestamp_millis();
        let b = zone.from_local_datetime(&(start + chrono::Duration::minutes((n as i64 + 1) * 30))).single()?.timestamp_millis();
        (b-a == 1_800_000).then_some((a,b))
    }).collect()
}

fn value(cached: &CachedAvailability, times: &[(i64,i64)], person: &str, slot: &tf::Vorschlag) -> Option<tf::Verfuegbarkeit> {
    if times.first()?.0 > slot.start || times.last()?.1 < slot.ende { return None; }
    let rows: Vec<_> = cached.values.iter().filter(|p| tf::normalisiere_adresse(&p.address) == person).collect();
    if rows.len() != 1 { return None; }
    if times.len() != rows[0].intervals.len() { return None; }
    let mut covered = slot.start;
    let mut severity = 0;
    let mut unknown = false;
    for (&(a,b),state) in times.iter().zip(&rows[0].intervals) {
        if a >= slot.ende || b <= slot.start { continue; }
        if a > covered { return None; }
        covered = covered.max(b);
        let weight = match state {
            Availability::Free => 0,
            Availability::WorkingElsewhere => 1,
            Availability::Tentative => 2,
            Availability::Busy => 3,
            Availability::Oof => 4,
            Availability::Unknown => { unknown = true; 0 }
        };
        severity = severity.max(weight);
    }
    if covered < slot.ende { return None; }
    // Bekannte Konflikte bleiben auch bei teilweise unbekanntem Kalender sicher.
    if unknown && severity < 3 { return Some(tf::Verfuegbarkeit::Unbekannt); }
    Some(match severity {
        4 => tf::Verfuegbarkeit::Abwesend,
        3 => tf::Verfuegbarkeit::Belegt,
        2 => tf::Verfuegbarkeit::Vorlaeufig,
        1 => tf::Verfuegbarkeit::Anderswo,
        _ => tf::Verfuegbarkeit::Frei,
    })
}

impl PollRuntime {
    pub fn rank(&self, svc: &M365Service, conn: &rusqlite::Connection, id: &str) -> Result<Vec<tf::Rang>,String> {
        // Ein geaendertes/gesperrtes Konto darf keine alten Kalenderdaten liefern.
        let owner = bound_account(svc,conn,id).ok().filter(|(a,_,_)| read_allowed(a,Capability::Verfuegbarkeit).is_ok())
            .map(|(a,key,_)| (a.integ.id,key));
        self.rank_cached(conn,id,owner.as_ref())
    }

    pub(super) fn rank_cached(&self, conn: &rusqlite::Connection, id: &str, owner: Option<&(String,String)>) -> Result<Vec<tf::Rang>,String> {
        let view = get(conn,id)?;
        let poll = view.poll.into_core();
        // In den App-Einstellungen existiert noch keine Kernzeit der einladenden Person.
        let mut free = tf::FreiBelegt::default();
        let snapshot: Vec<_> = {
            let cache = self.availability.lock().map_err(|_| "poll_lock")?;
            cache.values().filter(|c| c.until > Instant::now() && Some(&c.owner) == owner).cloned().collect()
        };
        let cached: Vec<_> = snapshot.iter()
            .filter_map(|c| intervals(&c.window).map(|times| (c,times))).collect();
        let mut missing = Vec::new();
        for slot in &poll.vorschlaege {
            for person in poll.teilnehmende.iter().filter(|p| p.intern) {
                let address = tf::normalisiere_adresse(&person.adresse);
                let mut latest = None;
                for (c,times) in &cached {
                    if let Some(wert) = value(c,times,&address,slot) {
                        match latest {
                            Some((until,_)) if until > c.until => {},
                            Some((until,old)) if until == c.until && old != wert => latest = Some((until,tf::Verfuegbarkeit::Unbekannt)),
                            _ => latest = Some((c.until,wert)),
                        }
                    }
                }
                if let Some((_,wert)) = latest.filter(|(_,v)| *v != tf::Verfuegbarkeit::Unbekannt) {
                    free.eintraege.push(tf::KalenderWert { person:address,vorschlag_nr:slot.nr,wert });
                } else { missing.push(slot.nr); }
            }
        }
        let mut ranks = tf::bewerte(&poll,&view.answers,&free);
        for rank in &mut ranks {
            if missing.contains(&rank.nr) {
                rank.begruendung.push_str(" Kalender-Verfügbarkeit fehlt oder ist unvollständig; für diese Personen zählen nur sichere/bestätigte Antworten.");
            }
        }
        Ok(ranks)
    }
}
