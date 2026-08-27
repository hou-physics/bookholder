use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::path::Path;
use std::time::Duration;

pub fn watch_projects(
    dir: &Path,
    mut on_change: impl FnMut() + Send + 'static,
) -> Result<Box<dyn std::any::Any + Send>, String> {
    let mut debouncer = new_debouncer(Duration::from_millis(500), move |res| {
        if let Ok(events) = res {
            let hit = {
                let evs: &Vec<notify_debouncer_mini::DebouncedEvent> = &events;
                evs.iter().any(|e| {
                    e.path.extension().map(|x| x == "jsonl").unwrap_or(false)
                        || e.path.is_dir()
                })
            };
            if hit {
                on_change();
            }
        }
    })
    .map_err(|e| e.to_string())?;
    debouncer
        .watcher()
        .watch(dir, RecursiveMode::Recursive)
        .map_err(|e| e.to_string())?;
    Ok(Box::new(debouncer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn fires_on_change_after_debounce() {
        let dir = tempfile::tempdir().unwrap();
        let counter = Arc::new(AtomicU32::new(0));
        let c2 = counter.clone();
        let _guard = watch_projects(dir.path(), move || {
            c2.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(300)); // 等 watcher 就绪
        std::fs::write(dir.path().join("a.jsonl"), "x\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while counter.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(counter.load(Ordering::SeqCst) >= 1, "watcher 未触发");
    }
}
