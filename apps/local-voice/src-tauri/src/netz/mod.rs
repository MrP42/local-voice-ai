//! Netzzugang für Downloads: Proxy-Wahl wie Windows und verständliche
//! Fehlermeldungen (lva#573).

pub mod befehle;
pub mod fehler;
pub mod proxy;
pub mod proxy_auth;
pub mod ziele;
#[cfg(test)]
mod firmentauglich_tests;
#[cfg(windows)]
pub mod sspi;
