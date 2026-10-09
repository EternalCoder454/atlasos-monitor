//! Which window has focus on Plasma, for [`Controller::set_focused`]: the
//! application in use is left alone. GNOME's uresourced shows that by raising
//! its weight, but it can't see focus under KWin. So a small KWin script
//! reports each focused window's process to this connection's unique name,
//! one a killed run's leftover script can never reach anyone else by (the
//! next run replaces that script). Off Plasma there is no KWin to load it,
//! and [`Watch::pid`] stays 0.
//!
//! [`Controller::set_focused`]: super::Controller::set_focused

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::oneshot;
use zbus::Connection;

const KWIN: &str = "org.kde.KWin";
const SCRIPTING_PATH: &str = "/Scripting";
const SCRIPTING_IF: &str = "org.kde.kwin.Scripting";
const SCRIPT_IF: &str = "org.kde.kwin.Script";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";
/// The script's name in KWin: one per session, whichever run loaded it.
const PLUGIN: &str = "net.eterneon.telamon.monitor.focus";
/// Where the script calls: the interface's name is spelt out again on
/// [`Focus`], which the macro needs as a literal.
const PATH: &str = "/Focus";
const IFACE: &str = "net.eterneon.telamon.monitor.Focus";
const TIMEOUT: Duration = Duration::from_secs(2);

/// KWin's scripting API (Plasma 6), calling `service`. The pid goes as a
/// string: callDBus would send a JS number as a double.
pub fn script(service: &str) -> String {
    format!(
        r#"function report(w) {{
    callDBus("{service}", "{PATH}", "{IFACE}", "Activated", w ? String(w.pid) : "0");
}}
workspace.windowActivated.connect(report);
report(workspace.activeWindow);
"#
    )
}

/// The unique bus name KWin has now, or `None` while there is none. Only the
/// KWin the script was loaded into may report: the session bus lets any
/// program of the session call any unique name (docs/SECURITY.md, "D-Bus
/// objects we serve").
type KwinOwner = Arc<Mutex<Option<String>>>;

struct Focus {
    pid: Arc<AtomicU32>,
    kwin: KwinOwner,
}

/// Whether a call from `sender` is KWin's, `owner` being KWin's unique name.
fn from_kwin(sender: Option<&str>, owner: Option<&str>) -> bool {
    matches!((sender, owner), (Some(s), Some(o)) if s == o)
}

#[zbus::interface(name = "net.eterneon.telamon.monitor.Focus")]
impl Focus {
    /// The process of the window that got focus, "0" for none. Reports from
    /// anyone but KWin are dropped.
    fn activated(&self, #[zbus(header)] header: zbus::message::Header<'_>, pid: &str) {
        let owner = self.kwin.lock().unwrap_or_else(PoisonError::into_inner);
        if !from_kwin(header.sender().map(|s| s.as_str()), owner.as_deref()) {
            return;
        }
        self.pid.store(pid.parse().unwrap_or(0), Ordering::Relaxed);
    }
}

/// KWin's reports, on a thread of its own that owns the connection. The
/// script is loaded again into a KWin that restarted. [`stop`](Self::stop)
/// (or dropping it) unloads it, waiting for KWin up to [`UNLOAD_WAIT`];
/// dropping it waits for that.
pub struct Watch {
    pid: Arc<AtomicU32>,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Watch {
    /// Starts watching. `dir` is where the script is written: Atlas's
    /// folder in the runtime directory ([`super::system::runtime_dir`]).
    pub fn start(dir: PathBuf) -> Self {
        let pid = Arc::new(AtomicU32::new(0));
        let (stop, stopped) = oneshot::channel();
        let reports = pid.clone();
        let thread = std::thread::Builder::new()
            .name("kwin-focus".into())
            .spawn(move || run(&dir, reports, stopped))
            .ok();
        Self {
            pid,
            stop: Some(stop),
            thread,
        }
    }

    /// The process of the window with focus, 0 for none or not known.
    pub fn pid(&self) -> u32 {
        self.pid.load(Ordering::Relaxed)
    }

    /// Asks the thread to unload the script and end, without waiting: so
    /// the caller can see to other things meanwhile, then drop this.
    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How often KWin's owner is checked, for a KWin that restarted.
const POLL: Duration = Duration::from_secs(10);
/// How long the way out waits for KWin to unload the script.
const UNLOAD_WAIT: Duration = Duration::from_secs(1);

fn run(dir: &Path, pid: Arc<AtomicU32>, mut stopped: oneshot::Receiver<()>) {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    // Reports are answered while this runs: the runtime is driven only here.
    rt.block_on(async {
        let kwin: KwinOwner = Arc::default();
        let conn = tokio::select! {
            conn = tokio::time::timeout(TIMEOUT, connect(pid.clone(), kwin.clone())) => conn.ok().flatten(),
            _ = &mut stopped => None,
        };
        let Some(conn) = conn else {
            return;
        };
        // The KWin (by unique name) the script was last loaded into, or
        // tried: a failed load isn't tried again until KWin changes.
        let mut tried: Option<String> = None;
        loop {
            tokio::select! {
                () = follow(&conn, dir, &pid, &kwin, &mut tried) => {}
                _ = &mut stopped => break,
            }
            tokio::select! {
                () = tokio::time::sleep(POLL) => {}
                _ = &mut stopped => break,
            }
        }
        if tried.is_some() {
            let _ = tokio::time::timeout(UNLOAD_WAIT, unload(&conn)).await;
        }
    });
}

/// A session connection serving [`PATH`].
async fn connect(pid: Arc<AtomicU32>, kwin: KwinOwner) -> Option<Connection> {
    zbus::connection::Builder::session()
        .ok()?
        .method_timeout(TIMEOUT)
        .serve_at(PATH, Focus { pid, kwin })
        .ok()?
        .build()
        .await
        .ok()
}

/// Loads the script into a KWin it isn't in yet. When KWin goes or is
/// replaced, the last report is stale: no window is known to have focus
/// until the new one reports.
async fn follow(
    conn: &Connection,
    dir: &Path,
    pid: &AtomicU32,
    kwin: &KwinOwner,
    tried: &mut Option<String>,
) {
    // Only the bus saying so means KWin is gone; a call that failed (a
    // timeout) leaves things as they were, the script perhaps loaded.
    let owner = match conn
        .call_method(Some(BUS), BUS_PATH, Some(BUS), "GetNameOwner", &(KWIN,))
        .await
    {
        Ok(m) => match m.body().deserialize::<String>() {
            Ok(owner) => Some(owner),
            Err(_) => return,
        },
        Err(zbus::Error::MethodError(name, _, _))
            if name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
        {
            None
        }
        Err(_) => return,
    };
    // Before the script is loaded: its first report comes at once.
    *kwin.lock().unwrap_or_else(PoisonError::into_inner) = owner.clone();
    if owner == *tried {
        return;
    }
    pid.store(0, Ordering::Relaxed);
    // Set first: stopped halfway, the way out still unloads.
    *tried = owner;
    if tried.is_some()
        && let Err(e) = load(conn, dir).await
    {
        log::info!("KWin did not take the focus script: {e}");
    }
}

/// Has KWin load and run the script.
async fn load(conn: &Connection, dir: &Path) -> Result<(), String> {
    let name = conn.unique_name().ok_or("no unique name")?;
    let path = write_script(dir, name.as_str()).map_err(|e| e.to_string())?;
    // A killed run's script first. KWin deletes a script later, so the load
    // waits for this answer (and would find the name taken without it).
    unload(conn).await.map_err(|e| e.to_string())?;
    let id = conn
        .call_method(
            Some(KWIN),
            SCRIPTING_PATH,
            Some(SCRIPTING_IF),
            "loadScript",
            &(path.to_str().ok_or("path not UTF-8")?, PLUGIN),
        )
        .await
        .map_err(|e| e.to_string())?
        .body()
        .deserialize::<i32>()
        .map_err(|e| e.to_string())?;
    if id < 0 {
        return Err("refused".into());
    }
    conn.call_method(
        Some(KWIN),
        format!("{SCRIPTING_PATH}/Script{id}").as_str(),
        Some(SCRIPT_IF),
        "run",
        &(),
    )
    .await
    .map(drop)
    .map_err(|e| e.to_string())
}

async fn unload(conn: &Connection) -> zbus::Result<()> {
    conn.call_method(
        Some(KWIN),
        SCRIPTING_PATH,
        Some(SCRIPTING_IF),
        "unloadScript",
        &(PLUGIN,),
    )
    .await
    .map(drop)
}

fn write_script(dir: &Path, service: &str) -> std::io::Result<PathBuf> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let path = dir.join("focus.js");
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)?
        .write_all(script(service).as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_kwin_may_report() {
        assert!(from_kwin(Some(":1.7"), Some(":1.7")));
        assert!(!from_kwin(Some(":1.8"), Some(":1.7")));
        // KWin not on the bus (or the sender unknown): nobody is believed.
        assert!(!from_kwin(Some(":1.7"), None));
        assert!(!from_kwin(None, Some(":1.7")));
        assert!(!from_kwin(None, None));
        assert!(!from_kwin(Some(""), Some(":1.7")));
    }

    /// On a real (session) bus: a stranger's report is dropped, KWin's is
    /// taken. Skipped where there is no session bus (CI's container has none;
    /// `dbus-run-session -- cargo test` gives one).
    #[test]
    fn on_the_bus_only_kwin_is_believed() {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
            eprintln!("no session bus; skipping");
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let pid = Arc::new(AtomicU32::new(0));
            let kwin: KwinOwner = Arc::default();
            let server = connect(pid.clone(), kwin.clone())
                .await
                .expect("session bus");
            let session = || async { zbus::connection::Builder::session()?.build().await };
            let real = session().await.unwrap();
            let stranger = session().await.unwrap();
            *kwin.lock().unwrap() = Some(real.unique_name().unwrap().to_string());
            let to = server.unique_name().unwrap().to_string();
            let report = |from: Connection, p: &'static str| {
                let to = to.clone();
                async move {
                    from.call_method(Some(to.as_str()), PATH, Some(IFACE), "Activated", &(p,))
                        .await
                        .unwrap();
                }
            };
            report(stranger, "4242").await;
            assert_eq!(pid.load(Ordering::Relaxed), 0, "a stranger moved the focus");
            report(real, "4243").await;
            assert_eq!(pid.load(Ordering::Relaxed), 4243);
        });
    }

    #[test]
    fn the_script_calls_what_is_served() {
        let s = script(":1.42");
        assert!(s.contains(r#"callDBus(":1.42", "#), "{s}");
        assert!(
            s.contains(r#""/Focus", "net.eterneon.telamon.monitor.Focus", "Activated""#),
            "{s}"
        );
    }
}
