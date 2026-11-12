//! Regelpruefung einer Planung (Konzept 3.1): kurze Besprechungen (25/50 Minuten),
//! Beginn zur vollen oder halben Stunde, ein Zielsatz, Agenda passt in die Dauer,
//! Termin im Arbeitszeitrahmen (Vorgabe 06-20 Uhr).
//!
//! Die Pruefung blockiert nichts: sie liefert Hinweise, die Oberflaeche zeigt sie.
//! Nur `Stufe::Fehler` verhindert die Uebergabe an Outlook (z. B. fehlender Titel).

use serde::{Deserialize, Serialize};

use crate::model::PlanInput;

/// Wie schwer ein Hinweis wiegt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Stufe {
    /// Uebergabe nicht moeglich.
    Fehler,
    /// Abweichung von der Empfehlung, Uebergabe trotzdem moeglich.
    Warnung,
    /// Nur zur Information.
    Info,
}

/// Ein Ergebnis der Pruefung. `code` ist stabil (Tests, Uebersetzung), `text` deutsch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Hinweis {
    pub code: String,
    pub stufe: Stufe,
    pub text: String,
}

/// Uhrzeit in der Ortszeit der Nutzerin bzw. des Nutzers. Die App rechnet sie aus
/// dem geplanten Beginn aus; das Crate kennt keine Zeitzonen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct LokaleZeit {
    pub stunde: u8,
    pub minute: u8,
}

impl LokaleZeit {
    fn minuten(&self) -> u32 {
        u32::from(self.stunde) * 60 + u32::from(self.minute)
    }
}

/// Einstellbare Grenzen. Die Vorgaben stammen aus dem Konzept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Regeln {
    /// Empfohlene Dauern in Minuten.
    pub empfohlene_dauern: Vec<u32>,
    /// Arbeitszeitrahmen: frueheste Beginnstunde und spaeteste Endstunde (lokal).
    pub rahmen_von_stunde: u8,
    pub rahmen_bis_stunde: u8,
    /// Laengste erlaubte Dauer, darueber ist es ein Fehler.
    pub max_dauer_min: u32,
}

impl Default for Regeln {
    fn default() -> Self {
        Regeln {
            empfohlene_dauern: vec![25, 50],
            rahmen_von_stunde: 6,
            rahmen_bis_stunde: 20,
            max_dauer_min: 8 * 60,
        }
    }
}

/// Hoechstlaenge des Titels (wie `meetings.title` der App).
pub const TITEL_MAX: usize = 200;

/// Prueft eine Planung. `beginn` ist optional: ohne Termin entfallen die Zeitregeln.
pub fn pruefen(plan: &PlanInput, beginn: Option<LokaleZeit>, regeln: &Regeln) -> Vec<Hinweis> {
    let mut out = Vec::new();
    let mut add = |code: &str, stufe: Stufe, text: String| {
        out.push(Hinweis {
            code: code.to_string(),
            stufe,
            text,
        })
    };

    let titel = plan.title.trim();
    if titel.is_empty() {
        add(
            "titel_leer",
            Stufe::Fehler,
            "Die Besprechung braucht einen Titel.".into(),
        );
    } else if titel.chars().count() > TITEL_MAX {
        add(
            "titel_zu_lang",
            Stufe::Fehler,
            format!("Der Titel ist länger als {TITEL_MAX} Zeichen."),
        );
    }

    if plan.goal_text.trim().is_empty() {
        add(
            "ziel_fehlt",
            Stufe::Warnung,
            "Formulieren Sie das Ziel in einem Satz: Was soll am Ende feststehen?".into(),
        );
    }
    if plan
        .expected_result
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        add(
            "ergebnis_fehlt",
            Stufe::Info,
            "Ein erwartetes Ergebnis hilft beim Abgleich im Protokoll.".into(),
        );
    }

    if plan.duration_min == 0 {
        add(
            "dauer_null",
            Stufe::Fehler,
            "Die Dauer muss größer als 0 sein.".into(),
        );
    } else if plan.duration_min > regeln.max_dauer_min {
        add(
            "dauer_zu_lang",
            Stufe::Fehler,
            format!("Die Dauer ist länger als {} Minuten.", regeln.max_dauer_min),
        );
    } else if !regeln.empfohlene_dauern.contains(&plan.duration_min) {
        let liste = regeln
            .empfohlene_dauern
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(" oder ");
        add(
            "dauer_unueblich",
            Stufe::Warnung,
            format!("Empfohlen sind {liste} Minuten, damit Zeit bis zum nächsten Termin bleibt."),
        );
    }

    if plan.agenda.is_empty() {
        add(
            "agenda_leer",
            Stufe::Warnung,
            "Ohne Agenda kann das Protokoll nicht nach Punkten gegliedert werden.".into(),
        );
    } else {
        if plan.agenda.iter().any(|a| a.topic.trim().is_empty()) {
            add(
                "agenda_punkt_leer",
                Stufe::Fehler,
                "Ein Agendapunkt hat kein Thema.".into(),
            );
        }
        if plan.agenda.iter().any(|a| a.minutes == 0) {
            add(
                "agenda_zeitbox_null",
                Stufe::Warnung,
                "Ein Agendapunkt hat keine Zeitbox.".into(),
            );
        }
        let summe: u32 = plan.agenda.iter().map(|a| a.minutes).sum();
        if summe > plan.duration_min && plan.duration_min > 0 {
            add(
                "agenda_zu_lang",
                Stufe::Warnung,
                format!(
                    "Die Agenda braucht {summe} Minuten, geplant sind {}.",
                    plan.duration_min
                ),
            );
        }
    }

    if plan.transcription_planned {
        add(
            "transkription_einwilligung",
            Stufe::Info,
            "Transkription geplant: Die Einladung bittet um Zustimmung, beim Start wird sie je Person bestätigt.".into(),
        );
    }

    if let Some(b) = beginn {
        if b.minute != 0 && b.minute != 30 {
            add(
                "beginn_krumm",
                Stufe::Info,
                "Empfohlen ist ein Beginn zur vollen oder halben Stunde.".into(),
            );
        }
        let start = b.minuten();
        let ende = start + plan.duration_min;
        let von = u32::from(regeln.rahmen_von_stunde) * 60;
        let bis = u32::from(regeln.rahmen_bis_stunde) * 60;
        if start < von || ende > bis {
            add(
                "ausserhalb_rahmen",
                Stufe::Warnung,
                format!(
                    "Der Termin liegt außerhalb des Arbeitszeitrahmens {:02}–{:02} Uhr.",
                    regeln.rahmen_von_stunde, regeln.rahmen_bis_stunde
                ),
            );
        }
    }

    out
}

/// Ist eine Uebergabe an Outlook erlaubt (kein Fehler)?
pub fn uebergabe_erlaubt(hinweise: &[Hinweis]) -> bool {
    !hinweise.iter().any(|h| h.stufe == Stufe::Fehler)
}
