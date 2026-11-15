//! Opt-in AniCraft UI freeze diagnostics. No cost beyond a OnceLock check unless
//! ANICRAFT_FREEZE_LOG is set before the application starts.
//! A watchdog writes from a separate thread, even when the UI stops responding.
use std::{
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{Arc, OnceLock, atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering}},
    thread,
    time::{Duration, Instant},
};

#[derive(Copy, Clone)]
#[repr(u8)]
pub(crate) enum Phase {
    Starting = 0,
    FrameBegin = 1,
    BridgeBegin = 2,
    BridgeDone = 3,
    RemoteDone = 4,
    PythonBegin = 5,
    PythonPump = 6,
    PythonDone = 7,
    WorkspaceBegin = 8,
    WorkspaceDone = 9,
    FrameDone = 10,
}
impl Phase {
    const fn label(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::FrameBegin => "frame.begin",
            Self::BridgeBegin => "bridge.import.begin",
            Self::BridgeDone => "bridge.import.done",
            Self::RemoteDone => "remote.poll.done",
            Self::PythonBegin => "python.tick.begin",
            Self::PythonPump => "python.qt_pump.begin",
            Self::PythonDone => "python.tick.done",
            Self::WorkspaceBegin => "workspace.begin",
            Self::WorkspaceDone => "workspace.done",
            Self::FrameDone => "frame.done",
        }
    }
    const fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::FrameBegin,
            2 => Self::BridgeBegin,
            3 => Self::BridgeDone,
            4 => Self::RemoteDone,
            5 => Self::PythonBegin,
            6 => Self::PythonPump,
            7 => Self::PythonDone,
            8 => Self::WorkspaceBegin,
            9 => Self::WorkspaceDone,
            10 => Self::FrameDone,
            _ => Self::Starting,
        }
    }
}
struct Monitor {
    path: PathBuf,
    origin: Instant,
    phase: AtomicU8,
    last_ms: AtomicU64,
    until_ms: AtomicU64,
    inside_frame: AtomicBool,
    started: AtomicBool,
}
static MONITOR: OnceLock<Option<Arc<Monitor>>> = OnceLock::new();

fn write(path: &PathBuf, line: &str) {
    if let Ok(mut output) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(output, "{line}");
    }
}

fn monitor() -> Option<&'static Arc<Monitor>> {
    MONITOR.get_or_init(|| {
        let path = std::env::var_os("ANICRAFT_FREEZE_LOG").filter(|p| !p.is_empty()).map(PathBuf::from)?;
        let value = Arc::new(Monitor {
            path,
            origin: Instant::now(),
            phase: AtomicU8::new(Phase::Starting as u8),
            last_ms: AtomicU64::new(0),
            until_ms: AtomicU64::new(0),
            inside_frame: AtomicBool::new(false),
            started: AtomicBool::new(false),
        });
        write(&value.path, "--- AniCraft freeze monitor started ---");
        let watching = Arc::clone(&value);
        let _ = thread::Builder::new().name("anicraft-freeze-watchdog".into()).spawn(move || {
            let mut last_report_ms = 0;
            loop {
                thread::sleep(Duration::from_millis(500));
                if !watching.started.load(Ordering::Relaxed) { continue; }
                let now = watching.origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                let since = now.saturating_sub(watching.last_ms.load(Ordering::Relaxed));
                let inside = watching.inside_frame.load(Ordering::Relaxed);
                let after_bridge = now <= watching.until_ms.load(Ordering::Relaxed);
                // A minimized or idle window may stop repainting. Only report a gap
                // outside the frame after a recent bridge update, not indefinitely.
                if (inside && since >= 2500 || after_bridge && since >= 4000)
                    && now.saturating_sub(last_report_ms) >= 3000 {
                    last_report_ms = now;
                    let phase = Phase::from_u8(watching.phase.load(Ordering::Relaxed));
                    write(&watching.path, &format!(
                        "{now} ms | WATCHDOG no UI progress for {since} ms | last={} | in_frame={inside} | after_bridge={after_bridge}",
                        phase.label(),
                    ));
                }
            }
        });
        Some(value)
    }).as_ref()
}

pub(crate) fn mark(phase: Phase) {
    let Some(m) = monitor() else { return };
    let ms = m.origin.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    m.phase.store(phase as u8, Ordering::Relaxed);
    m.last_ms.store(ms, Ordering::Relaxed);
    m.started.store(true, Ordering::Relaxed);
    match phase {
        Phase::FrameBegin => m.inside_frame.store(true, Ordering::Relaxed),
        Phase::FrameDone => m.inside_frame.store(false, Ordering::Relaxed),
        Phase::BridgeDone => {
            m.until_ms.store(ms.saturating_add(60_000), Ordering::Relaxed);
            write(&m.path, &format!("{ms} ms | bridge.import.done | next 60s monitored"));
        },
        Phase::BridgeBegin => write(&m.path, &format!("{ms} ms | bridge.import.begin")),
        _ => {},
    }
}
