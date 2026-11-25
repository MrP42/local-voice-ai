//! lva#806: kurzlebige Aktivitaet im Prozess, unabhaengig von Nutzerpausen.
use std::sync::atomic::{AtomicUsize, Ordering};

static TTS_STARTS: AtomicUsize = AtomicUsize::new(0);

pub struct ActivityLease<'a>(&'a AtomicUsize);

fn begin(counter: &AtomicUsize) -> ActivityLease<'_> {
    counter.fetch_add(1, Ordering::AcqRel);
    ActivityLease(counter)
}

pub fn tts_starting() -> ActivityLease<'static> {
    begin(&TTS_STARTS)
}

pub fn tts_start_active() -> bool {
    TTS_STARTS.load(Ordering::Acquire) != 0
}

impl Drop for ActivityLease<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_leases_release_on_error_and_unwind() {
        let counter = AtomicUsize::new(0);
        let first = begin(&counter);
        let _ = std::panic::catch_unwind(|| {
            let _second = begin(&counter);
            panic!("synthetic");
        });
        assert_eq!(counter.load(Ordering::Acquire), 1);
        drop(first);
        assert_eq!(counter.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn cancelled_health_future_releases_its_lease() {
        let counter = AtomicUsize::new(0);
        let mut future = Box::pin(async {
            let _lease = begin(&counter);
            std::future::pending::<()>().await;
        });
        tokio::select! {
            _ = &mut future => panic!("synthetic pending future completed"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(10)) => {}
        }
        assert_eq!(counter.load(Ordering::Acquire), 1);
        drop(future);
        assert_eq!(counter.load(Ordering::Acquire), 0);
    }
}
