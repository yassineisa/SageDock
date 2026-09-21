//! Shared helper for shelling out to console tools (`wsl.exe`, `powershell.exe`, and
//! later WSL-internal commands). Centralized here so every call site gets the same two
//! behaviors instead of reimplementing them ad hoc:
//!
//! - No flashing console window. A GUI app spawning a console tool otherwise briefly
//!   shows a terminal window, which is exactly the "you shouldn't need a terminal"
//!   experience the product spec rules out.
//! - Output decoding that doesn't assume a codepage. Windows console tools are
//!   inconsistent about whether piped output comes back as UTF-16LE or the system's
//!   ANSI codepage; guessing wrong turns readable text into mojibake in the "Show
//!   details" panel. This only affects human-readable debug text — no decision in this
//!   codebase parses that text to make a choice (see the module docs in `wsl.rs`).

use std::io::Read;
use std::os::windows::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Avoids a console window flashing on screen when spawning a console subprocess from
/// this GUI app. See `CreateProcess` / `dwCreationFlags` in the Windows API docs.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub struct ProcessResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub combined_output: String,
}

/// Runs `program` with `args`, hidden, and returns its exit status plus best-effort
/// decoded output. Returns `Ok(None)` if the program itself couldn't be found/started —
/// callers treat that the same as "not available" rather than an internal error.
pub fn run_hidden(program: &str, args: &[&str]) -> std::io::Result<Option<ProcessResult>> {
    run_hidden_timeout(program, args, Duration::from_secs(45))
}

/// Drains both pipes concurrently with bounded retained output. A stuck child cannot
/// hold an application command forever. Long installation operations choose a deadline.
pub fn run_hidden_timeout(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<Option<ProcessResult>> {
    let spawned = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    fn drain(mut pipe: impl Read + Send + 'static) -> std::sync::mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut tail = Vec::new();
            let mut buf = [0; 8192];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > 65536 {
                    tail.drain(..tail.len() - 65536);
                }
            }
            let _ = tx.send(tail);
        });
        rx
    }
    let stdout = drain(child.stdout.take().unwrap());
    let stderr = drain(child.stderr.take().unwrap());
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut,
                "The operation exceeded its time limit. It may still be finishing inside Windows or the computing environment; check its status before retrying."));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok(Some(to_result(Output {
        status,
        stdout: stdout
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_default(),
        stderr: stderr
            .recv_timeout(Duration::from_secs(1))
            .unwrap_or_default(),
    })))
}

fn to_result(output: Output) -> ProcessResult {
    let mut combined = decode_console_bytes(&output.stdout);
    let stderr = decode_console_bytes(&output.stderr);
    if !stderr.trim().is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr);
    }

    ProcessResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        combined_output: combined,
    }
}

/// Heuristic UTF-16LE vs UTF-8 decode: ASCII text encoded as UTF-16LE has a null byte
/// after every ASCII code unit, which valid UTF-8 text essentially never contains.
fn decode_console_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let looks_utf16 = bytes.len() >= 4
        && bytes
            .iter()
            .skip(1)
            .step_by(2)
            .take(16)
            .filter(|&&b| b == 0)
            .count()
            > 4;

    if looks_utf16 && bytes.len().is_multiple_of(2) {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_plain_utf8() {
        assert_eq!(decode_console_bytes(b"hello world"), "hello world");
    }

    #[test]
    fn decodes_utf16le_ascii() {
        let utf16: Vec<u8> = "hello"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        assert_eq!(decode_console_bytes(&utf16), "hello");
    }

    #[test]
    fn empty_input_is_empty_string() {
        assert_eq!(decode_console_bytes(&[]), "");
    }
}
