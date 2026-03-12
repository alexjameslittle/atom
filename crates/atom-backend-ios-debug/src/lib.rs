use std::fs::File;
use std::io::{BufReader, Read, Write};
#[cfg(unix)]
use std::os::fd::FromRawFd;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use atom_backends::{
    DebugBacktrace, DebugBreakpoint, DebugFrame, DebugSourceLocation, DebugStop, DebugThread,
    DebuggerKind,
};
use atom_ffi::{AtomError, AtomErrorCode, AtomResult};
use camino::{Utf8Path, Utf8PathBuf};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
const LLDB_DEFAULT_PROMPT: &str = "(lldb) ";
const LLDB_PROMPT: &str = "__ATOM_LLDB_PROMPT__";

pub struct IosAttachOptions {
    pub executable_path: Utf8PathBuf,
    pub pid: u32,
    pub source_map_prefix: Option<String>,
}

pub struct IosLldbClient {
    child: Child,
    stdin: File,
    output_rx: Receiver<u8>,
    running: bool,
}

impl IosLldbClient {
    /// # Errors
    ///
    /// Returns an error if LLDB could not be started or attached to the target process.
    pub fn attach(repo_root: &Utf8Path, options: &IosAttachOptions) -> AtomResult<Self> {
        let (master, slave) = open_pty_pair()?;
        let stdout = master.try_clone().map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to clone LLDB PTY master: {error}"),
            )
        })?;
        let stdin = master;
        let child = Command::new("/usr/bin/lldb")
            .args(["--no-lldbinit"])
            .current_dir(repo_root)
            .stdin(Stdio::from(slave.try_clone().map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to clone LLDB PTY slave: {error}"),
                )
            })?))
            .stdout(Stdio::from(slave.try_clone().map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to clone LLDB PTY slave: {error}"),
                )
            })?))
            .stderr(Stdio::from(slave))
            .spawn()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to start LLDB: {error}"),
                )
            })?;

        let (output_tx, output_rx) = mpsc::channel();
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            let mut byte = [0_u8; 1];
            loop {
                match stdout.read(&mut byte) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if output_tx.send(byte[0]).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        let mut client = Self {
            child,
            stdin,
            output_rx,
            running: false,
        };
        let _ = client.read_until_marker(LLDB_DEFAULT_PROMPT, COMMAND_TIMEOUT)?;
        let _ = client.run_command(&format!("settings set prompt {LLDB_PROMPT}"))?;
        let _ = client.run_command("settings set auto-confirm true")?;
        if let Some(prefix) = options.source_map_prefix.as_deref() {
            let _ = client.run_command(&format!(
                "settings append target.source-map {} {}",
                lldb_quote(prefix),
                lldb_quote(repo_root.as_str())
            ))?;
        }
        let _ = client.run_command(&format!(
            "target create {}",
            lldb_quote(options.executable_path.as_str())
        ))?;
        let _ = client.run_command(&format!("process attach --pid {}", options.pid))?;
        let _ = client.drain_pending_output(Duration::from_millis(100));
        let _ = client.process_status()?;
        let _ = client.drain_pending_output(Duration::from_millis(100));
        Ok(client)
    }

    /// # Errors
    ///
    /// Returns an error if the breakpoint could not be created.
    pub fn set_breakpoint(
        &mut self,
        location: &DebugSourceLocation,
    ) -> AtomResult<DebugBreakpoint> {
        self.ensure_paused("set a breakpoint")?;
        let output = self.run_command(&format!(
            "breakpoint set --file {} --line {}",
            lldb_quote(&location.file),
            location.line
        ))?;
        parse_breakpoint(location, &output)
    }

    /// # Errors
    ///
    /// Returns an error if the breakpoint could not be deleted.
    pub fn clear_breakpoint(&mut self, breakpoint_id: &str) -> AtomResult<()> {
        self.ensure_paused("clear a breakpoint")?;
        let _ = self.run_command(&format!("breakpoint delete {breakpoint_id}"))?;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the process does not stop before the timeout expires.
    pub fn wait_for_stop(&mut self, timeout_ms: Option<u64>) -> AtomResult<DebugStop> {
        if self.running {
            let timeout = timeout_ms.map_or(DEFAULT_WAIT_TIMEOUT, Duration::from_millis);
            let deadline = Instant::now() + timeout;
            loop {
                if Instant::now() >= deadline {
                    return Err(AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        "timed out while waiting for LLDB to report a stop",
                    ));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                let output = self.read_until_prompt_or_stop(remaining)?;
                let stop = parse_stop(&output);
                if stop.reason != "unknown"
                    || !stop
                        .description
                        .as_deref()
                        .is_some_and(|description| description.contains(" is running."))
                {
                    self.running = false;
                    return Ok(stop);
                }
            }
        }
        self.process_status()
    }

    /// # Errors
    ///
    /// Returns an error if thread metadata could not be captured.
    pub fn threads(&mut self) -> AtomResult<Vec<DebugThread>> {
        self.ensure_paused("inspect threads")?;
        let output = self.run_command("thread list")?;
        Ok(parse_threads(&output))
    }

    /// # Errors
    ///
    /// Returns an error if backtrace metadata could not be captured.
    pub fn backtrace(&mut self, thread_id: Option<&str>) -> AtomResult<DebugBacktrace> {
        self.ensure_paused("inspect backtraces")?;
        let output = self.run_command("thread backtrace all")?;
        parse_backtrace(&output, thread_id)
    }

    /// # Errors
    ///
    /// Returns an error if the process could not be interrupted.
    pub fn pause(&mut self) -> AtomResult<DebugStop> {
        if self.running {
            let status = Command::new("/bin/kill")
                .args(["-INT", &self.child.id().to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| {
                    AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        format!("failed to interrupt LLDB: {error}"),
                    )
                })?;
            if !status.success() {
                return Err(AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    "LLDB interrupt command failed",
                ));
            }
            let output = self.read_until_prompt_or_stop(COMMAND_TIMEOUT)?;
            self.running = false;
            let mut stop = parse_stop(&output);
            if stop.reason == "unknown" {
                "paused".clone_into(&mut stop.reason);
            }
            return Ok(stop);
        }
        let mut stop = self.process_status()?;
        if stop.reason == "unknown" {
            "paused".clone_into(&mut stop.reason);
        }
        Ok(stop)
    }

    /// # Errors
    ///
    /// Returns an error if the process could not be resumed.
    pub fn resume(&mut self) -> AtomResult<()> {
        if self.running {
            return Ok(());
        }
        self.send_line("continue")?;
        let _ = self.read_until_marker(LLDB_PROMPT, COMMAND_TIMEOUT)?;
        self.running = true;
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the debugger could not be detached or terminated cleanly.
    pub fn shutdown(&mut self) -> AtomResult<()> {
        if self.running {
            let _ = self.pause();
        }
        let _ = self.run_command("process detach");
        let _ = self.send_line("quit");
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match self.child.try_wait().map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to poll LLDB during shutdown: {error}"),
                )
            })? {
                Some(_) => return Ok(()),
                None if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return Ok(());
                }
                None => thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    fn ensure_paused(&self, action: &str) -> AtomResult<()> {
        if self.running {
            Err(AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                format!("debugger must be paused before it can {action}"),
            ))
        } else {
            Ok(())
        }
    }

    fn process_status(&mut self) -> AtomResult<DebugStop> {
        let output = self.run_command("process status")?;
        Ok(parse_stop(&output))
    }

    fn run_command(&mut self, command: &str) -> AtomResult<String> {
        self.send_line(command)?;
        let output = self.read_until_marker(LLDB_PROMPT, COMMAND_TIMEOUT)?;
        Ok(normalize_command_output(&output, command))
    }

    fn send_line(&mut self, command: &str) -> AtomResult<()> {
        self.stdin
            .write_all(command.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to write LLDB command: {error}"),
                )
            })
    }

    fn read_until_marker(&mut self, marker: &str, timeout: Duration) -> AtomResult<String> {
        let deadline = Instant::now() + timeout;
        let marker_bytes = marker.as_bytes();
        let mut buffer = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err(AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "timed out while waiting for LLDB output:\n{}",
                        String::from_utf8_lossy(&buffer)
                    ),
                ));
            }
            match self.output_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(byte) => {
                    buffer.push(byte);
                    if buffer.ends_with(marker_bytes) {
                        let trimmed = &buffer[..buffer.len() - marker_bytes.len()];
                        return Ok(String::from_utf8_lossy(trimmed).replace('\r', ""));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(status) = self.child.try_wait().map_err(|error| {
                        AtomError::new(
                            AtomErrorCode::ExternalToolFailed,
                            format!("failed to poll LLDB process: {error}"),
                        )
                    })? {
                        return Err(AtomError::new(
                            AtomErrorCode::ExternalToolFailed,
                            format!(
                                "LLDB exited unexpectedly with status {status}:\n{}",
                                String::from_utf8_lossy(&buffer)
                            ),
                        ));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        "LLDB output stream closed unexpectedly",
                    ));
                }
            }
        }
    }

    fn read_until_prompt_or_stop(&mut self, timeout: Duration) -> AtomResult<String> {
        let deadline = Instant::now() + timeout;
        let marker_bytes = LLDB_PROMPT.as_bytes();
        let mut buffer = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err(AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "timed out while waiting for LLDB output:\n{}",
                        String::from_utf8_lossy(&buffer)
                    ),
                ));
            }
            let wait = if buffer.is_empty() {
                deadline.saturating_duration_since(Instant::now())
            } else {
                Duration::from_millis(200).min(deadline.saturating_duration_since(Instant::now()))
            };
            match self.output_rx.recv_timeout(wait) {
                Ok(byte) => {
                    buffer.push(byte);
                    if buffer.ends_with(marker_bytes) {
                        return Ok(String::from_utf8_lossy(&buffer).into_owned());
                    }
                }
                Err(RecvTimeoutError::Timeout) if output_contains_stop(&buffer) => {
                    return Ok(String::from_utf8_lossy(&buffer).into_owned());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        format!(
                            "LLDB exited unexpectedly while waiting for output:\n{}",
                            String::from_utf8_lossy(&buffer)
                        ),
                    ));
                }
            }
        }
    }

    fn drain_pending_output(&mut self, idle_timeout: Duration) -> String {
        let mut buffer = Vec::new();
        while let Ok(byte) = self.output_rx.recv_timeout(idle_timeout) {
            buffer.push(byte);
        }
        String::from_utf8_lossy(&buffer).into_owned()
    }
}

fn normalize_command_output(output: &str, command: &str) -> String {
    let mut lines = output
        .lines()
        .map(str::trim_end)
        .filter(|line| *line != LLDB_PROMPT)
        .collect::<Vec<_>>();
    if let Some(first) = lines.first().copied() {
        let first = first.strip_prefix(LLDB_PROMPT).unwrap_or(first);
        if first == command {
            lines.remove(0);
        } else if let Some(stripped) = first.strip_prefix(command) {
            lines[0] = stripped.trim_start();
        } else {
            lines[0] = first;
        }
    }
    lines.join("\n").trim().to_owned()
}

fn parse_breakpoint(location: &DebugSourceLocation, output: &str) -> AtomResult<DebugBreakpoint> {
    let line = output
        .lines()
        .find(|line| line.contains("Breakpoint "))
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("LLDB did not report a breakpoint id:\n{output}"),
            )
        })?;
    let id = line
        .split("Breakpoint ")
        .nth(1)
        .and_then(|value| value.split(':').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("LLDB breakpoint output was missing an id:\n{output}"),
            )
        })?;
    let resolved = line
        .rsplit_once(" at ")
        .and_then(|(_, suffix)| {
            suffix
                .split_once(',')
                .map(|(left, _)| left)
                .or(Some(suffix))
        })
        .and_then(parse_file_line);
    Ok(DebugBreakpoint {
        debugger: DebuggerKind::Native,
        file: location.file.clone(),
        line: location.line,
        id: id.to_owned(),
        resolved_file: resolved.as_ref().map(|(file, _)| file.clone()),
        resolved_line: resolved.map(|(_, line)| line),
    })
}

fn parse_stop(output: &str) -> DebugStop {
    let mut stop = DebugStop {
        debugger: DebuggerKind::Native,
        reason: "unknown".to_owned(),
        description: (!output.trim().is_empty()).then(|| output.trim().to_owned()),
        thread_id: None,
        thread_name: None,
        breakpoint_id: None,
        file: None,
        line: None,
    };
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(thread_id) = parse_lldb_thread_id(trimmed) {
            stop.thread_id = Some(thread_id);
        }
        if let Some(reason) = trimmed.split("stop reason = ").nth(1)
            && let Some(reason) = reason.split(',').next()
        {
            let reason = reason.trim();
            if let Some(id) = reason.strip_prefix("breakpoint ").map(str::trim) {
                "breakpoint".clone_into(&mut stop.reason);
                stop.breakpoint_id = Some(id.to_owned());
            } else if reason.eq_ignore_ascii_case("signal SIGSTOP") {
                "paused".clone_into(&mut stop.reason);
            } else {
                stop.reason = reason.to_ascii_lowercase().replace(' ', "_");
            }
        }
        if let Some((file, line)) = trimmed
            .rsplit_once(" at ")
            .and_then(|(_, suffix)| parse_file_line(suffix))
        {
            stop.file = Some(file);
            stop.line = Some(line);
        }
    }
    stop
}

fn parse_threads(output: &str) -> Vec<DebugThread> {
    output.lines().filter_map(parse_thread_line).collect()
}

fn parse_thread_line(line: &str) -> Option<DebugThread> {
    let trimmed = line.trim();
    if !trimmed.contains("thread #") {
        return None;
    }
    let selected = trimmed.starts_with('*');
    let trimmed = trimmed.trim_start_matches('*').trim();
    let thread_id = parse_lldb_thread_id(trimmed)?;
    let name = trimmed
        .split(',')
        .find_map(|segment| {
            segment
                .trim()
                .strip_prefix("name = ")
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            trimmed.split(',').find_map(|segment| {
                segment
                    .trim()
                    .strip_prefix("queue = ")
                    .map(ToOwned::to_owned)
            })
        });
    let state = trimmed
        .split("stop reason = ")
        .nth(1)
        .and_then(|segment| segment.split(',').next())
        .map(|value| value.trim().to_owned())
        .or_else(|| {
            trimmed
                .split(',')
                .next()
                .and_then(|segment| segment.contains("stopped").then(|| "stopped".to_owned()))
        });
    Some(DebugThread {
        debugger: DebuggerKind::Native,
        id: thread_id,
        name,
        state,
        selected,
    })
}

fn parse_backtrace(output: &str, requested_thread_id: Option<&str>) -> AtomResult<DebugBacktrace> {
    let mut thread_id = None;
    let mut thread_name = None;
    let mut frames = Vec::new();
    let mut capturing = false;
    let mut saw_selected_thread = false;
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.contains("thread #") {
            let current_id = parse_lldb_thread_id(trimmed);
            let is_selected = line.trim_start().starts_with("* thread #");
            capturing = if let Some(requested_thread_id) = requested_thread_id {
                current_id.as_deref() == Some(requested_thread_id)
            } else if is_selected {
                saw_selected_thread = true;
                true
            } else {
                !saw_selected_thread && frames.is_empty()
            };
            if capturing {
                thread_id = current_id;
                thread_name = trimmed
                    .split(',')
                    .find_map(|segment| {
                        segment
                            .trim()
                            .strip_prefix("name = ")
                            .map(ToOwned::to_owned)
                    })
                    .or_else(|| {
                        trimmed.split(',').find_map(|segment| {
                            segment
                                .trim()
                                .strip_prefix("queue = ")
                                .map(ToOwned::to_owned)
                        })
                    });
                frames.clear();
            }
            continue;
        }
        if !capturing || !trimmed.starts_with("frame #") {
            continue;
        }
        if let Some(frame) = parse_frame_line(trimmed) {
            frames.push(frame);
        }
    }
    if frames.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            "LLDB did not return a backtrace for the requested thread",
        ));
    }
    Ok(DebugBacktrace {
        debugger: DebuggerKind::Native,
        thread_id,
        thread_name,
        frames,
    })
}

fn parse_frame_line(line: &str) -> Option<DebugFrame> {
    let index = line
        .strip_prefix("frame #")?
        .split(':')
        .next()?
        .trim()
        .parse::<usize>()
        .ok()?;
    let after_colon = line.split_once(':')?.1.trim();
    let (function_part, file_part) = after_colon
        .rsplit_once(" at ")
        .map_or((after_colon, None), |(left, right)| (left, Some(right)));
    let mut module = None;
    let function = if let Some((module_name, symbol)) = function_part.split_once('`') {
        module = Some(module_name.trim().to_owned());
        symbol.trim().to_owned()
    } else {
        function_part.trim().to_owned()
    };
    let (file, line) = file_part
        .and_then(parse_file_line)
        .map_or((None, None), |(file, line)| (Some(file), Some(line)));
    Some(DebugFrame {
        index,
        function,
        module,
        file,
        line,
    })
}

fn output_contains_stop(buffer: &[u8]) -> bool {
    let output = String::from_utf8_lossy(buffer);
    output.contains("stop reason = ") || output.contains("Target 0: (app) stopped.")
}

fn parse_lldb_thread_id(line: &str) -> Option<String> {
    let fragment = line.split("thread #").nth(1)?;
    let digits = fragment
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    (!digits.is_empty()).then_some(digits)
}

fn parse_file_line(fragment: &str) -> Option<(String, u32)> {
    let fragment = fragment.trim();
    let fragment = fragment.split(':').collect::<Vec<_>>();
    if fragment.len() < 2 {
        return None;
    }
    let line = fragment
        .get(fragment.len().wrapping_sub(2))?
        .trim()
        .parse::<u32>()
        .ok()?;
    let file = fragment[..fragment.len() - 2].join(":");
    Some((file, line))
}

fn lldb_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(unix)]
fn open_pty_pair() -> AtomResult<(File, File)> {
    use std::ffi::{c_char, c_int, c_void};

    unsafe extern "C" {
        fn openpty(
            amaster: *mut c_int,
            aslave: *mut c_int,
            name: *mut c_char,
            termp: *const c_void,
            winp: *const c_void,
        ) -> c_int;
    }

    let mut master_fd = 0;
    let mut slave_fd = 0;
    let status = unsafe {
        openpty(
            &raw mut master_fd,
            &raw mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if status != 0 {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "failed to allocate LLDB PTY: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    let master = unsafe { File::from_raw_fd(master_fd) };
    let slave = unsafe { File::from_raw_fd(slave_fd) };
    Ok((master, slave))
}

#[cfg(test)]
mod tests {
    use atom_backends::DebuggerKind;

    use super::{
        LLDB_PROMPT, normalize_command_output, parse_backtrace, parse_breakpoint, parse_stop,
        parse_threads,
    };

    #[test]
    fn normalize_command_output_strips_prompt_and_echo() {
        let output = format!("{LLDB_PROMPT}thread list\nProcess 1 stopped\n{LLDB_PROMPT}");
        assert_eq!(
            normalize_command_output(&output, "thread list"),
            "Process 1 stopped"
        );
    }

    #[test]
    fn breakpoint_parsing_reads_id_and_location() {
        let breakpoint = parse_breakpoint(
            &atom_backends::DebugSourceLocation {
                file: "/tmp/demo.swift".to_owned(),
                line: 54,
            },
            "Breakpoint 3: where = app`main + 12 at /tmp/demo.swift:54:9, address = 0x1234",
        )
        .expect("breakpoint should parse");
        assert_eq!(breakpoint.id, "3");
        assert_eq!(breakpoint.resolved_line, Some(54));
    }

    #[test]
    fn stop_parsing_reads_breakpoint_reason() {
        let stop = parse_stop(
            "Process 123 stopped\n* thread #1, queue = 'com.apple.main-thread', stop reason = breakpoint 4.1\n    frame #0: 0x1 app`main at /tmp/demo.swift:54:9",
        );
        assert_eq!(stop.debugger, DebuggerKind::Native);
        assert_eq!(stop.reason, "breakpoint");
        assert_eq!(stop.breakpoint_id.as_deref(), Some("4.1"));
        assert_eq!(stop.thread_id.as_deref(), Some("1"));
        assert_eq!(stop.line, Some(54));
    }

    #[test]
    fn threads_parsing_keeps_selected_marker() {
        let threads = parse_threads(
            "Process 123 stopped\n* thread #1, queue = 'com.apple.main-thread', stop reason = breakpoint 4.1\n  thread #2, name = worker, stop reason = signal SIGSTOP",
        );
        assert_eq!(threads.len(), 2);
        assert!(threads[0].selected);
        assert_eq!(threads[1].name.as_deref(), Some("worker"));
    }

    #[test]
    fn backtrace_parsing_extracts_frames() {
        let backtrace = parse_backtrace(
            "* thread #1, queue = 'com.apple.main-thread', stop reason = breakpoint 1.1\n  frame #0: 0x1 app`main at /tmp/demo.swift:54:9\n  frame #1: 0x2 app`next at /tmp/demo.swift:40:3\n  thread #2, name = worker, stop reason = signal SIGSTOP\n  frame #0: 0x3 app`worker at /tmp/worker.swift:10:1",
            Some("1"),
        )
        .expect("backtrace should parse");
        assert_eq!(backtrace.thread_id.as_deref(), Some("1"));
        assert_eq!(backtrace.frames.len(), 2);
        assert_eq!(backtrace.frames[0].line, Some(54));
    }
}
