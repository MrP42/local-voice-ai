use crate::{KontoFehler, ProfilDatei, speicher::KontoSpeicher};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Handle der Änderungsbeobachtung; der Polling-Thread stoppt beim Drop.
pub struct Beobachter {
    stopp: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Beobachter {
    pub(crate) fn starten<F>(
        speicher: KontoSpeicher,
        intervall: Duration,
        callback: F,
    ) -> Result<Self, KontoFehler>
    where
        F: Fn(&ProfilDatei) + Send + 'static,
    {
        let mut zuletzt = speicher.lesen()?.generation;
        let stopp = Arc::new((Mutex::new(false), Condvar::new()));
        let stopp_thread = Arc::clone(&stopp);
        let thread = std::thread::spawn(move || {
            let (sperre, signal) = &*stopp_thread;
            loop {
                let Ok(gehalten) = sperre.lock() else { return };
                let Ok((gehalten, _)) = signal.wait_timeout_while(gehalten, intervall, |s| !*s)
                else {
                    return;
                };
                if *gehalten {
                    return;
                }
                drop(gehalten);
                // Lesefehler (etwa kurzer Zugriffskonflikt unter Windows): nächste Runde.
                if let Ok(datei) = speicher.lesen()
                    && datei.generation != zuletzt
                {
                    zuletzt = datei.generation;
                    callback(&datei);
                }
            }
        });
        Ok(Self {
            stopp,
            thread: Some(thread),
        })
    }
}

impl Drop for Beobachter {
    fn drop(&mut self) {
        let (sperre, signal) = &*self.stopp;
        if let Ok(mut s) = sperre.lock() {
            *s = true;
        }
        signal.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
