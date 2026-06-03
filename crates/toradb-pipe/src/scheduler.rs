
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::store::{now_unix_secs, PipeStore};
pub type RunningSet = Arc<Mutex<HashSet<String>>>;

pub struct RunGuard {
    running: RunningSet,
    id: String,
}

impl RunGuard {
    pub fn claim(running: &RunningSet, id: &str) -> Option<RunGuard> {
        let mut set = running.lock().ok()?;
        if set.contains(id) {
            return None;
        }
        set.insert(id.to_string());
        Some(RunGuard {
            running: running.clone(),
            id: id.to_string(),
        })
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.running.lock() {
            set.remove(&self.id);
        }
    }
}

pub fn spawn_scheduler(
    store: Arc<Mutex<PipeStore>>,
    running: RunningSet,
    on_due: impl Fn(String) + Send + Sync + 'static,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        loop {
            tick.tick().await;
            let now = now_unix_secs();
            let due: Vec<String> = {
                let Ok(s) = store.lock() else { continue };
                s.enabled_scheduled()
                    .into_iter()
                    .filter(|p| {
                        let interval = p.schedule.as_ref().map(|s| s.interval_secs).unwrap_or(0);
                        if interval == 0 {
                            return false;
                        }
                        now.saturating_sub(s.last_run_started(&p.id)) >= interval
                    })
                    .map(|p| p.id)
                    .collect()
            };
            for id in due {
                if running.lock().map(|s| s.contains(&id)).unwrap_or(true) {
                    continue;
                }
                on_due(id);
            }
        }
    });
}
