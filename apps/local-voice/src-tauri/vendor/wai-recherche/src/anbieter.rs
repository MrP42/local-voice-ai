//! Suchanbieter. Standard ist ein selbst betriebenes SearXNG (ADR-0026 E2). Kein automatischer
//! Anbieterwechsel: bei Ausfall kommt `AnbieterNichtErreichbar`, die Person waehlt.
//!
//! Hinweis: Auch ueber SearXNG verlassen Suchanfragen die Umgebung (an die befragten Suchmaschinen).

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::budget::BudgetZaehler;
use crate::fehler::RechercheFehler;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suchanfrage {
    pub text: String,
    pub sprache: Option<String>,
    /// Obergrenze der zurueckgegebenen Treffer.
    pub max_treffer: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Treffer {
    pub titel: String,
    pub url: String,
    pub auszug: String,
    pub datum: Option<String>,
}

#[async_trait]
pub trait Suchanbieter: Send + Sync {
    /// Unbudgetierte Anbieter-Implementierung; Aufrufer nutzen ausschliesslich `suchen_mit_budget`.
    async fn suchen(&self, anfrage: &Suchanfrage) -> Result<Vec<Treffer>, RechercheFehler>;

    /// Verbucht eine Suche im Budget (Ueberschreitung = Fehler) und fuehrt sie dann aus.
    async fn suchen_mit_budget(
        &self,
        anfrage: &Suchanfrage,
        budget: &mut BudgetZaehler,
    ) -> Result<Vec<Treffer>, RechercheFehler> {
        budget.suche()?;
        self.suchen(anfrage).await
    }
}

/// Hoechstlaenge des Suchtexts in Zeichen; laengere Texte sind ein Fehler (kein stilles Kuerzen).
pub const MAX_SUCHTEXT: usize = 512;

const MAX_ANTWORT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SearxngAnbieter {
    basis: Url,
    client: reqwest::Client,
}

impl SearxngAnbieter {
    /// `basis` z. B. `http://searxng.intern:8080`; JSON-Ausgabe muss am Dienst freigeschaltet sein.
    pub fn neu(basis: &str, zeitlimit: Duration) -> Result<Self, RechercheFehler> {
        let basis = Url::parse(basis).map_err(|_| RechercheFehler::UngueltigeUrl)?;
        if !matches!(basis.scheme(), "http" | "https") {
            return Err(RechercheFehler::SchemaNichtErlaubt);
        }
        let client = reqwest::Client::builder()
            .timeout(zeitlimit)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("WAI-Recherche/{}", crate::VERSION))
            .build()
            .map_err(|_| RechercheFehler::AnbieterNichtErreichbar)?;
        Ok(Self { basis, client })
    }

    fn such_url(&self, anfrage: &Suchanfrage) -> Result<Url, RechercheFehler> {
        if anfrage.text.chars().count() > MAX_SUCHTEXT {
            return Err(RechercheFehler::AnfrageZuLang);
        }
        let mut u = self.basis.clone();
        {
            let mut seg = u
                .path_segments_mut()
                .map_err(|_| RechercheFehler::UngueltigeUrl)?;
            seg.pop_if_empty().push("search");
        }
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("format", "json");
            q.append_pair("q", &anfrage.text);
            if let Some(s) = &anfrage.sprache {
                q.append_pair("language", s);
            }
        }
        Ok(u)
    }
}

#[async_trait]
impl Suchanbieter for SearxngAnbieter {
    async fn suchen(&self, anfrage: &Suchanfrage) -> Result<Vec<Treffer>, RechercheFehler> {
        let url = self.such_url(anfrage)?;
        let mut antwort = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|_| RechercheFehler::AnbieterNichtErreichbar)?;
        if !antwort.status().is_success() {
            return Err(RechercheFehler::AnbieterNichtErreichbar);
        }
        let mut body = Vec::new();
        while let Some(teil) = antwort
            .chunk()
            .await
            .map_err(|_| RechercheFehler::AnbieterNichtErreichbar)?
        {
            if body.len() + teil.len() > MAX_ANTWORT_BYTES {
                return Err(RechercheFehler::ZuGross);
            }
            body.extend_from_slice(&teil);
        }
        let text = String::from_utf8_lossy(&body);
        parse_searxng(&text, anfrage.max_treffer)
    }
}

#[derive(Deserialize)]
struct SearxAntwort {
    #[serde(default)]
    results: Vec<SearxErgebnis>,
}

#[derive(Deserialize)]
struct SearxErgebnis {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    #[serde(rename = "publishedDate")]
    published_date: Option<String>,
}

/// Wandelt die SearXNG-JSON-Antwort in Treffer um: nur http(s)-URLs, ohne Dubletten, auf `max` gekuerzt.
pub fn parse_searxng(json: &str, max: usize) -> Result<Vec<Treffer>, RechercheFehler> {
    let a: SearxAntwort =
        serde_json::from_str(json).map_err(|_| RechercheFehler::AntwortUngueltig)?;
    let mut gesehen = std::collections::HashSet::new();
    let mut aus = Vec::new();
    for r in a.results {
        if aus.len() >= max {
            break;
        }
        let (Some(url), Some(titel)) = (r.url, r.title) else {
            continue;
        };
        let titel = titel.trim().to_string();
        let ok = Url::parse(&url)
            .map(|u| matches!(u.scheme(), "http" | "https"))
            .unwrap_or(false);
        if !ok || titel.is_empty() || !gesehen.insert(url.clone()) {
            continue;
        }
        aus.push(Treffer {
            titel,
            url,
            auszug: r.content.unwrap_or_default().trim().to_string(),
            datum: r.published_date.filter(|d| !d.trim().is_empty()),
        });
    }
    Ok(aus)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/searxng.json");

    #[test]
    fn fixture_wird_geparst_und_gekuerzt() {
        let t = parse_searxng(FIXTURE, 10).unwrap();
        // 5 Eintraege: 1 ohne url, 1 mit javascript:, 1 Dublette -> 2 gueltige
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].titel, "Beispiel Amtsblatt");
        assert_eq!(t[0].datum.as_deref(), Some("2026-03-01T00:00:00"));
        assert_eq!(t[1].datum, None);
        assert_eq!(parse_searxng(FIXTURE, 1).unwrap().len(), 1);
        assert_eq!(parse_searxng(FIXTURE, 0).unwrap().len(), 0);
    }

    #[test]
    fn ungueltiges_json() {
        assert!(matches!(
            parse_searxng("<html>", 5),
            Err(RechercheFehler::AntwortUngueltig)
        ));
        assert!(parse_searxng("{}", 5).unwrap().is_empty());
    }

    #[test]
    fn such_url_aufbau() {
        let a = SearxngAnbieter::neu("http://searx.intern:8080/", Duration::from_secs(5)).unwrap();
        let u = a
            .such_url(&Suchanfrage {
                text: "a b&c".into(),
                sprache: Some("de".into()),
                max_treffer: 5,
            })
            .unwrap();
        assert_eq!(u.path(), "/search");
        let q: Vec<_> = u.query_pairs().into_owned().collect();
        assert!(q.contains(&("format".into(), "json".into())));
        assert!(q.contains(&("q".into(), "a b&c".into())));
        assert!(q.contains(&("language".into(), "de".into())));
    }

    #[test]
    fn basis_schema_geprueft() {
        assert!(SearxngAnbieter::neu("ftp://x", Duration::from_secs(1)).is_err());
        assert!(SearxngAnbieter::neu("kein url", Duration::from_secs(1)).is_err());
    }

    #[test]
    fn suchtext_laenge() {
        let a = SearxngAnbieter::neu("http://s:1", Duration::from_secs(1)).unwrap();
        let mk = |n: usize| Suchanfrage {
            text: "a".repeat(n),
            sprache: None,
            max_treffer: 1,
        };
        assert!(a.such_url(&mk(512)).is_ok());
        assert!(matches!(
            a.such_url(&mk(513)),
            Err(RechercheFehler::AnfrageZuLang)
        ));
    }

    struct Fest;
    #[async_trait]
    impl Suchanbieter for Fest {
        async fn suchen(&self, _: &Suchanfrage) -> Result<Vec<Treffer>, RechercheFehler> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn suchen_mit_budget_zaehlt_und_bricht_ab() {
        use crate::budget::Budget;
        let mut b = BudgetZaehler::neu(Budget {
            searches: 1,
            fetches: 0,
            tokens: 0,
            seconds: 60,
        });
        let q = Suchanfrage {
            text: "x".into(),
            sprache: None,
            max_treffer: 1,
        };
        assert!(Fest.suchen_mit_budget(&q, &mut b).await.is_ok());
        assert!(matches!(
            Fest.suchen_mit_budget(&q, &mut b).await,
            Err(RechercheFehler::BudgetErreicht("searches"))
        ));
    }

    #[tokio::test]
    async fn ausfall_ist_fehler_ohne_wechsel() {
        // Port 1 auf Loopback: nichts lauscht
        let a = SearxngAnbieter::neu("http://127.0.0.1:1", Duration::from_secs(2)).unwrap();
        let r = a
            .suchen(&Suchanfrage {
                text: "x".into(),
                sprache: None,
                max_treffer: 3,
            })
            .await;
        assert!(matches!(r, Err(RechercheFehler::AnbieterNichtErreichbar)));
    }
}
