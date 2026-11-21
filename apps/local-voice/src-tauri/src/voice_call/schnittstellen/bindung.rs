use super::anfrage::{baue_adresse, pruefe_argumente};
use super::kodierung::{kanonisch, Wert};
use super::{fehler, objekt};

// Absichtlich keine zweite Hash-Implementierung: dieselbe Funktion bindet die Freigaben.
pub use crate::managers::integrations::approvals::args_hash as bindung;

pub fn fuer_aufruf(e: &Wert, a: &Wert) -> Result<Wert, Wert> {
    let art = e["art"].s();
    if a.map().is_none() {
        return Err(fehler("argumente_typ", None));
    }
    let (faehigkeit, ziel, argumente) = match art {
        "app" => (
            e["app"]["faehigkeit"].s().to_owned(),
            String::new(),
            a.clone(),
        ),
        "mcp" => (
            "schnittstelle.mcp".to_owned(),
            format!("{}/{}", e["mcp"]["server"].s(), e["mcp"]["werkzeug"].s()),
            a.clone(),
        ),
        _ => {
            let werte = pruefe_argumente(e, a)?;
            let adresse = baue_adresse(e, &werte)?;
            let methode = if art == "web" {
                "GET"
            } else {
                e["http"]["methode"].s()
            };
            (
                format!("schnittstelle.{art}"),
                format!("{methode} {adresse}"),
                werte,
            )
        }
    };
    aus_teilen(e["id"].s(), &faehigkeit, &ziel, &argumente)
}

pub(super) fn aus_teilen(
    id: &str,
    faehigkeit: &str,
    ziel: &str,
    argumente: &Wert,
) -> Result<Wert, Wert> {
    let argumente = kanonisch(argumente).map_err(|_| fehler("argumente_typ", None))?;
    let hash = bindung(id, faehigkeit, ziel, &argumente);
    Ok(objekt([
        ("faehigkeit", faehigkeit.into()),
        ("ziel", ziel.into()),
        ("argumente", argumente.into()),
        ("bindung", hash.into()),
    ]))
}
