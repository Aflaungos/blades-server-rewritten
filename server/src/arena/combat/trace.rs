//! Per-match recording of what the server sent each player.
//!
//! WHY THIS EXISTS INSTEAD OF A PACKET CAPTURE (#152).
//!
//! The desync report is "human-vs-human matches are very desynced" — the two
//! clients render different fights. The instinct was to re-enable the mitm
//! capture rig and record a match. That is the wrong tool and it was the wrong
//! instinct: the capture rig exists to observe **Bethesda's** server, which we
//! did not control. We run the server the client talks to now. If we want to
//! know what each player was told, we can simply write it down.
//!
//! WHY HERE. `MatchInstance::on_c2s` and `on_tick` are the only two ways bytes
//! leave the combat engine, and both return `Vec<(slot, bytes)>` — the complete
//! outbound stream, already split per player. One seam, both directions, nothing
//! to miss. The alternative (tracing at each of the dozen `broadcast_*` sites)
//! is exactly the shape of bug this codebase keeps paying for.
//!
//! WHAT A DESYNC LOOKS LIKE IN THE OUTPUT. Two files, one per player slot,
//! ordered. If the server sent the two players different things, the streams
//! diverge and the first differing line names the message. If it sent them the
//! same things, the divergence is client-side and the trace says so — which is
//! just as useful and is not otherwise knowable.
//!
//! OFF BY DEFAULT. Enabled only when `ARENA_TRACE_DIR` is set, resolved once.
//! Disabled, this costs one atomic load per call. Nothing here may panic or fail
//! a match: a trace that takes the server down is worse than no trace.

use std::io::Write;
use std::path::PathBuf;

/// The directory to write traces into, or `None` when tracing is off.
fn trace_dir() -> Option<&'static PathBuf> {
    static DIR: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let raw = std::env::var("ARENA_TRACE_DIR").ok()?;
        if raw.trim().is_empty() {
            return None;
        }
        let path = PathBuf::from(raw);
        if let Err(e) = std::fs::create_dir_all(&path) {
            log::warn!("[arena-trace] cannot create {}: {e} — tracing stays off", path.display());
            return None;
        }
        log::info!("[arena-trace] recording matches into {}", path.display());
        Some(path)
    })
    .as_ref()
}

pub fn enabled() -> bool {
    trace_dir().is_some()
}

/// The first two bytes carry the routing the protocol is keyed on: byte 0 is the
/// marker and byte 1 is the CARRIER, not the game message id. Recorded
/// separately so a reader does not have to re-derive the thing every analysis of
/// this protocol has got wrong at least once.
fn carrier_and_marker(bytes: &[u8]) -> (Option<u8>, Option<u8>) {
    (bytes.first().copied(), bytes.get(1).copied())
}

/// Record one message. `direction` is "s2c" or "c2s"; `slot` is the player it
/// was sent to (s2c) or came from (c2s).
pub fn record(session_id: &str, direction: &str, slot: usize, bytes: &[u8]) {
    let Some(dir) = trace_dir() else { return };
    record_into(dir, session_id, direction, slot, bytes);
}

/// The body of [`record`], with the destination passed in.
///
/// Split out so the writing can be tested for real. `trace_dir` is a `OnceLock`
/// resolved from the environment, so a test that set `ARENA_TRACE_DIR` would
/// race every other test in the process and prove nothing — and a trace feature
/// that is never shown to WRITE is exactly the kind of thing that ships inert.
fn record_into(dir: &std::path::Path, session_id: &str, direction: &str, slot: usize, bytes: &[u8]) {
    // A session id reaches the filesystem, so allow only characters that cannot
    // walk out of the directory.
    let safe: String = session_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    let safe = if safe.is_empty() { "unknown".to_string() } else { safe };
    let (marker, carrier) = carrier_and_marker(bytes);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let head: String = bytes.iter().take(24).map(|b| format!("{b:02x}")).collect();
    let line = format!(
        "{{\"t\":{now},\"dir\":\"{direction}\",\"slot\":{slot},\"len\":{},\
         \"marker\":{},\"carrier\":{},\"head\":\"{head}\"}}\n",
        bytes.len(),
        marker.map(|m| m.to_string()).unwrap_or_else(|| "null".into()),
        carrier.map(|c| c.to_string()).unwrap_or_else(|| "null".into()),
    );

    // Best effort throughout. A failed trace write must never disturb a match.
    let path = dir.join(format!("{safe}.jsonl"));
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            let _ = f.write_all(line.as_bytes());
        }
        Err(e) => {
            log::debug!("[arena-trace] {}: {e}", path.display());
        }
    }
}

/// Record a whole outbound batch, which is how the engine produces them.
pub fn record_outbound(session_id: &str, out: &[(usize, Vec<u8>)]) {
    if !enabled() {
        return;
    }
    for (slot, bytes) in out {
        record(session_id, "s2c", *slot, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Off unless asked for. The cost of the check, and the guarantee that a
    /// production server writes nothing unless someone turned it on.
    #[test]
    fn tracing_is_off_without_the_env_var() {
        // ARENA_TRACE_DIR is not set in the test environment.
        assert!(!enabled(), "tracing must default to off");
        // and the recording calls must be harmless no-ops
        record("session", "s2c", 0, &[1, 2, 3]);
        record_outbound("session", &[(0, vec![1, 2, 3])]);
    }

    /// The trace must actually write, and one line per message. A feature that
    /// is only ever tested in its OFF state can ship doing nothing.
    #[test]
    fn it_writes_one_line_per_message() {
        let dir = std::env::temp_dir().join(format!("arena-trace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let session = "sess-123";
        record_into(&dir, session, "c2s", 0, &[0xBE, 0x35, 0x07]);
        record_into(&dir, session, "s2c", 1, &[0xBE, 0x36]);
        let body = std::fs::read_to_string(dir.join("sess-123.jsonl")).expect("trace file");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "one line per message");
        assert!(lines[0].contains("\"dir\":\"c2s\"") && lines[0].contains("\"slot\":0"));
        assert!(lines[0].contains("\"carrier\":53"), "carrier is the SECOND byte: {}", lines[0]);
        assert!(lines[0].contains("\"len\":3"));
        assert!(lines[1].contains("\"dir\":\"s2c\"") && lines[1].contains("\"slot\":1"));
        for l in &lines {
            serde_json::from_str::<serde_json::Value>(l).expect("each line is valid JSON");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A session id must not be able to walk out of the trace directory.
    #[test]
    fn a_hostile_session_id_cannot_escape_the_directory() {
        let dir = std::env::temp_dir().join(format!("arena-trace-esc-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        record_into(&dir, "../../etc/passwd", "s2c", 0, &[1]);
        let written: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(written.len(), 1, "exactly one file, inside the directory");
        let name = written[0].file_name();
        let name = name.to_string_lossy();
        assert!(!name.contains(".."), "path traversal survived: {name}");
        assert!(name.ends_with(".jsonl"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The marker/carrier split is the thing readers get wrong; pin it.
    #[test]
    fn the_second_byte_is_the_carrier() {
        assert_eq!(carrier_and_marker(&[0xBE, 0x35, 0x01]), (Some(0xBE), Some(0x35)));
        assert_eq!(carrier_and_marker(&[0xBE]), (Some(0xBE), None));
        assert_eq!(carrier_and_marker(&[]), (None, None));
    }
}
