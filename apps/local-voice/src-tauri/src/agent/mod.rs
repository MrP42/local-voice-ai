//! Goal Lokaler Agent (#68). C1: Werkzeugwahl im Schema-Modus und ihr Eval
//! (`--eval-agent`). C2: die Laufzeit (`runtime`: Schema-Anfrage, ein Wiederholversuch,
//! Rueckfall `no_action`), die Datumsaufloesung im Code (`dates`) und `agent.extract`
//! (`extract`: To-dos, Fristen, Entscheidungen mit Segment-Belegen). `agent.route` folgt in C3.

pub mod dates;
pub mod eval;
pub mod extract;
pub mod runtime;
pub mod schema;
#[cfg(test)]
pub(crate) mod test_support;
