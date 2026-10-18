//! When the alternate playlist takes over and when the screen dims.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

const PROCESS_POLL: Duration = Duration::from_secs(5);
/// Linux truncates a process's `comm` name to 15 bytes.
const COMM_LEN: usize = 15;

pub(super) fn local_minute() -> u32 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    now.hour() * 60 + now.minute()
}

fn normalized(name: &str) -> String {
    name.trim().to_lowercase()
}

fn truncated(name: &str) -> &str {
    let mut end = name.len().min(COMM_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

/// True when any process's short name matches one of `names`, ignoring case.
/// Names longer than the kernel's 15-byte short name are confirmed against
/// the executable file name when it can be read.
pub(super) fn any_process_running(proc_root: &Path, names: &[String]) -> bool {
    let names: Vec<String> = names.iter().map(|name| normalized(name)).collect();
    if names.is_empty() {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return false;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_str()
            .is_some_and(|pid| pid.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let directory = entry.path();
        let Ok(comm) = std::fs::read_to_string(directory.join("comm")) else {
            continue;
        };
        let comm = normalized(&comm);
        for name in &names {
            if truncated(name) != comm {
                continue;
            }
            // A full-length short name may be a truncated longer one, so the
            // executable name tells programs sharing their first 15 bytes apart.
            if name.len() <= COMM_LEN || executable_name(&directory).is_none_or(|exe| &exe == name)
            {
                return true;
            }
        }
    }
    false
}

fn executable_name(process: &Path) -> Option<String> {
    let cmdline = std::fs::read(process.join("cmdline")).ok()?;
    let first = cmdline.split(|&b| b == 0).next()?;
    let program = String::from_utf8_lossy(first);
    let base = normalized(program.rsplit(['/', '\\']).next()?);
    (!base.is_empty()).then_some(base)
}

/// Polls for the listed programs on its own thread, so the renderer only
/// reads a flag.
pub(super) struct ProcessWatch {
    running: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ProcessWatch {
    pub(super) fn start(names: Vec<String>) -> Option<Self> {
        if names.is_empty() {
            return None;
        }
        let running = Arc::new(AtomicBool::new(any_process_running(
            Path::new("/proc"),
            &names,
        )));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::Builder::new()
            .name("playlist-process-watch".into())
            .spawn({
                let running = running.clone();
                let stop = stop.clone();
                move || {
                    while !stop.load(Ordering::Relaxed) {
                        thread::park_timeout(PROCESS_POLL);
                        if stop.load(Ordering::Relaxed) {
                            break;
                        }
                        running.store(
                            any_process_running(Path::new("/proc"), &names),
                            Ordering::Relaxed,
                        );
                    }
                }
            })
            .map_err(|error| tracing::warn!("playlist cannot watch for programs: {error}"))
            .ok()?;
        Some(Self {
            running,
            stop,
            thread: Some(thread),
        })
    }

    pub(super) fn running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl Drop for ProcessWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_proc() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let add = |pid: &str, comm: &str, cmdline: &[u8]| {
            let directory = root.path().join(pid);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("comm"), format!("{comm}\n")).unwrap();
            std::fs::write(directory.join("cmdline"), cmdline).unwrap();
        };
        add("1", "systemd", b"/sbin/init\0");
        add(
            "4242",
            "Cyberpunk2077.e",
            b"Z:\\games\\Cyberpunk2077.exe\0--launcher\0",
        );
        add("777", "steam", b"/home/six/.steam/steam\0");
        std::fs::create_dir(root.path().join("self")).unwrap();
        root
    }

    #[test]
    fn programs_match_by_short_or_executable_name_ignoring_case() {
        let root = fake_proc();
        let running = |name: &str| any_process_running(root.path(), &[name.to_string()]);
        assert!(running("STEAM"));
        assert!(running("cyberpunk2077.exe"));
        assert!(running("Cyberpunk2077.exe "));
        assert!(!running("cyberpunk"));
        assert!(
            !running("Cyberpunk2077.elf"),
            "a program sharing the first 15 bytes matched"
        );
        assert!(!running("discord"));
        assert!(!any_process_running(root.path(), &[]));
    }

    #[test]
    fn watch_stops_promptly_when_dropped() {
        let watch = ProcessWatch::start(vec!["no-such-program-xyz".into()]).unwrap();
        assert!(!watch.running());
        let started = std::time::Instant::now();
        drop(watch);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
