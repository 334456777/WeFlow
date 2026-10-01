//! Change notifications for the desktop app: `wcdb_start_monitor_pipe` opens a named pipe (Windows) or a Unix socket
//! and writes one JSON line per change it sees in the open accounts' databases:
//!
//! ```text
//! {"action":"session_change","db":"session","table":"SessionTable","dbPath":"…/session.db"}
//! {"action":"message_change","db":"message","dbPath":"…/message_0.db"}
//! {"action":"contact_change","db":"contact","dbPath":"…/contact.db"}
//! ```
//!
//! The desktop app treats the `action` as the change type and refreshes what depends on it. Changes are found by
//! polling the database and `-wal` file sizes and modification times once a second (the files are read-only to us).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

type Client = Box<dyn Write + Send>;

struct Running {
    name: String,
    stop: Arc<AtomicBool>,
}

fn running() -> &'static Mutex<Option<Running>> {
    static RUNNING: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    RUNNING.get_or_init(Default::default)
}

/// The pipe or socket name of the running monitor.
pub fn name() -> Option<String> {
    running().lock().ok()?.as_ref().map(|r| r.name.clone())
}

/// Starts the monitor (once); returns its pipe or socket name.
pub fn start(watch: impl Fn() -> Vec<PathBuf> + Send + 'static) -> std::io::Result<String> {
    let mut guard = running()
        .lock()
        .map_err(|_| std::io::Error::other("monitor lock poisoned"))?;
    if let Some(r) = guard.as_ref() {
        return Ok(r.name.clone());
    }
    let stop = Arc::new(AtomicBool::new(false));
    let clients: Arc<Mutex<Vec<Client>>> = Arc::default();
    let name = listen(stop.clone(), clients.clone())?;
    let watcher_stop = stop.clone();
    std::thread::spawn(move || watch_loop(watch, clients, watcher_stop));
    *guard = Some(Running {
        name: name.clone(),
        stop,
    });
    Ok(name)
}

pub fn stop() {
    let Some(r) = running().lock().ok().and_then(|mut g| g.take()) else {
        return;
    };
    r.stop.store(true, Ordering::Relaxed);
    unblock(&r.name);
}

// ── watching ──

type Stamp = (u64, Option<SystemTime>, u64, Option<SystemTime>);

fn stamp(db: &Path) -> Stamp {
    let meta = |p: &Path| {
        std::fs::metadata(p)
            .ok()
            .map(|m| (m.len(), m.modified().ok()))
    };
    let (a, b) = meta(db).unwrap_or((0, None));
    let mut wal = db.as_os_str().to_os_string();
    wal.push("-wal");
    let (c, d) = meta(Path::new(&wal)).unwrap_or((0, None));
    (a, b, c, d)
}

/// The databases worth watching under each `db_storage` folder.
pub fn databases(db_storage: &Path) -> Vec<PathBuf> {
    let mut out = vec![
        db_storage.join("session/session.db"),
        db_storage.join("contact/contact.db"),
    ];
    if let Ok(entries) = std::fs::read_dir(db_storage.join("message")) {
        let mut shards: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with("message_")
                        && n.ends_with(".db")
                        && !n.contains("fts")
                        && !n.contains("resource")
                })
            })
            .collect();
        shards.sort();
        out.extend(shards);
    }
    out
}

/// The change line for a database that changed.
pub fn event(db: &Path) -> String {
    let path = db.to_string_lossy();
    let file = db.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let v = if file == "session.db" {
        serde_json::json!({ "action": "session_change", "db": "session", "table": "SessionTable", "dbPath": path })
    } else if file == "contact.db" {
        serde_json::json!({ "action": "contact_change", "db": "contact", "dbPath": path })
    } else {
        serde_json::json!({ "action": "message_change", "db": "message", "dbPath": path })
    };
    v.to_string()
}

fn watch_loop(
    watch: impl Fn() -> Vec<PathBuf>,
    clients: Arc<Mutex<Vec<Client>>>,
    stop: Arc<AtomicBool>,
) {
    let mut known: HashMap<PathBuf, Stamp> = HashMap::new();
    while !stop.load(Ordering::Relaxed) {
        let mut lines: Vec<String> = Vec::new();
        for db in watch() {
            let now = stamp(&db);
            match known.insert(db.clone(), now) {
                Some(before) if before != now => lines.push(event(&db)),
                _ => {}
            }
        }
        if !lines.is_empty() {
            if let Ok(mut list) = clients.lock() {
                let payload = lines.join("\n") + "\n";
                list.retain_mut(|c| {
                    c.write_all(payload.as_bytes())
                        .and_then(|_| c.flush())
                        .is_ok()
                });
            }
        }
        std::thread::sleep(Duration::from_millis(1000));
    }
}

// ── transport ──

#[cfg(unix)]
fn listen(stop: Arc<AtomicBool>, clients: Arc<Mutex<Vec<Client>>>) -> std::io::Result<String> {
    use std::os::unix::net::UnixListener;
    let path = std::env::temp_dir().join(format!("weflow_monitor_{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    let name = path.to_string_lossy().to_string();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    if let Ok(mut list) = clients.lock() {
                        list.push(Box::new(stream));
                    }
                }
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        let _ = std::fs::remove_file(&path);
    });
    Ok(name)
}

#[cfg(unix)]
fn unblock(_name: &str) {}

#[cfg(windows)]
mod pipe {
    use std::io::{Error, Result, Write};
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{WriteFile, PIPE_ACCESS_DUPLEX};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    pub struct Instance(pub HANDLE);
    // SAFETY: a pipe handle can be used from any thread; each instance is owned by one client entry.
    unsafe impl Send for Instance {}

    impl Instance {
        pub fn create(name: &str) -> Result<Self> {
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            // SAFETY: `wide` is NUL-terminated and outlives the call.
            let h = unsafe {
                CreateNamedPipeW(
                    wide.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    64 * 1024,
                    4 * 1024,
                    0,
                    std::ptr::null(),
                )
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(Error::last_os_error());
            }
            Ok(Self(h))
        }

        /// Blocks until a client connects.
        pub fn accept(&self) -> Result<()> {
            // SAFETY: valid pipe handle, synchronous mode.
            let ok = unsafe { ConnectNamedPipe(self.0, std::ptr::null_mut()) };
            if ok != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
                Ok(())
            } else {
                Err(Error::last_os_error())
            }
        }
    }

    impl Write for Instance {
        fn write(&mut self, buf: &[u8]) -> Result<usize> {
            let mut written = 0u32;
            // SAFETY: valid handle; `buf` is readable for its length.
            let ok = unsafe {
                WriteFile(
                    self.0,
                    buf.as_ptr(),
                    buf.len() as u32,
                    &mut written,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return Err(Error::last_os_error());
            }
            Ok(written as usize)
        }
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl Drop for Instance {
        fn drop(&mut self) {
            // SAFETY: the handle is owned by this value.
            unsafe {
                DisconnectNamedPipe(self.0);
                CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
fn listen(stop: Arc<AtomicBool>, clients: Arc<Mutex<Vec<Client>>>) -> std::io::Result<String> {
    let name = format!(r"\\.\pipe\weflow_monitor_{}", std::process::id());
    // the first instance exists before this returns, so a client can connect right away
    let first = pipe::Instance::create(&name)?;
    let pipe_name = name.clone();
    std::thread::spawn(move || {
        let mut next = Some(first);
        while !stop.load(Ordering::Relaxed) {
            let instance = match next
                .take()
                .map(Ok)
                .unwrap_or_else(|| pipe::Instance::create(&pipe_name))
            {
                Ok(i) => i,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            };
            if instance.accept().is_ok() && !stop.load(Ordering::Relaxed) {
                if let Ok(mut list) = clients.lock() {
                    list.push(Box::new(instance));
                }
            }
        }
    });
    Ok(name)
}

/// Wakes the accept loop that is blocked waiting for a client.
#[cfg(windows)]
fn unblock(name: &str) {
    let _ = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_name_the_kind_of_database() {
        let v: serde_json::Value =
            serde_json::from_str(&event(Path::new("/a/db_storage/session/session.db"))).unwrap();
        assert_eq!(
            (v["action"].as_str(), v["table"].as_str()),
            (Some("session_change"), Some("SessionTable"))
        );
        let v: serde_json::Value =
            serde_json::from_str(&event(Path::new("/a/db_storage/message/message_3.db"))).unwrap();
        assert_eq!(v["action"], "message_change");
        let v: serde_json::Value =
            serde_json::from_str(&event(Path::new("/a/db_storage/contact/contact.db"))).unwrap();
        assert_eq!(v["action"], "contact_change");
    }

    #[cfg(unix)]
    #[test]
    fn clients_receive_a_line_when_a_watched_file_changes() {
        use std::io::{BufRead, BufReader};
        let dir = std::env::temp_dir().join(format!("weflow-monitor-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let db = dir.join("message/message_0.db");
        std::fs::write(&db, b"one").unwrap();
        let watched = db.clone();
        let name = start(move || vec![watched.clone()]).unwrap();
        assert_eq!(super::name().as_deref(), Some(name.as_str()));
        let stream = std::os::unix::net::UnixStream::connect(&name).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        std::thread::sleep(Duration::from_millis(1500)); // the watcher records the first stamp
        std::fs::write(&db, b"one two").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["action"], "message_change");
        assert_eq!(v["dbPath"], db.to_string_lossy().as_ref());
        stop();
        assert!(super::name().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
