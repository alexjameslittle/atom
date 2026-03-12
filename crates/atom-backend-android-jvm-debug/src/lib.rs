use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::net::TcpListener;
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

pub struct AndroidJvmAttachOptions {
    pub serial: String,
    pub pid: u32,
    pub source_root: Utf8PathBuf,
    pub generated_kotlin_jar: Utf8PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedJvmBreakpoint {
    class_name: String,
    line: u32,
}

pub struct AndroidJvmClient {
    child: Child,
    stdin: File,
    output_rx: Receiver<u8>,
    repo_root: Utf8PathBuf,
    serial: String,
    local_port: u16,
    generated_kotlin_jar: Utf8PathBuf,
    running: bool,
    current_stop: Option<DebugStop>,
    breakpoints: BTreeMap<String, Vec<ResolvedJvmBreakpoint>>,
}

enum JdbPrompt {
    Running,
    Stopped(String),
}

impl AndroidJvmClient {
    /// # Errors
    ///
    /// Returns an error if adb forwarding or JDB attach fails.
    pub fn attach(repo_root: &Utf8Path, options: &AndroidJvmAttachOptions) -> AtomResult<Self> {
        let local_port = free_tcp_port()?;
        run_command(
            repo_root,
            "adb",
            &[
                "-s".to_owned(),
                options.serial.clone(),
                "forward".to_owned(),
                format!("tcp:{local_port}"),
                format!("jdwp:{}", options.pid),
            ],
        )?;

        let (master, slave) = open_pty_pair()?;
        let stdout = master.try_clone().map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to clone JDB PTY master: {error}"),
            )
        })?;
        let stdin = master;
        let child = Command::new("/usr/bin/jdb")
            .args([
                "-sourcepath",
                options.source_root.as_str(),
                "-attach",
                &format!("127.0.0.1:{local_port}"),
            ])
            .current_dir(repo_root)
            .stdin(Stdio::from(slave.try_clone().map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to clone JDB PTY slave: {error}"),
                )
            })?))
            .stdout(Stdio::from(slave.try_clone().map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to clone JDB PTY slave: {error}"),
                )
            })?))
            .stderr(Stdio::from(slave))
            .spawn()
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to start JDB: {error}"),
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
            repo_root: repo_root.to_owned(),
            serial: options.serial.clone(),
            local_port,
            generated_kotlin_jar: options.generated_kotlin_jar.clone(),
            running: true,
            current_stop: None,
            breakpoints: BTreeMap::new(),
        };
        let (_, prompt) = client.read_until_prompt(COMMAND_TIMEOUT)?;
        client.running = matches!(prompt, JdbPrompt::Running);
        Ok(client)
    }

    /// # Errors
    ///
    /// Returns an error if the breakpoint could not be resolved or created.
    pub fn set_breakpoint(
        &mut self,
        location: &DebugSourceLocation,
    ) -> AtomResult<DebugBreakpoint> {
        let key = breakpoint_key(location);
        if self.breakpoints.contains_key(&key) {
            return Ok(DebugBreakpoint {
                debugger: DebuggerKind::Jvm,
                file: location.file.clone(),
                line: location.line,
                id: key,
                resolved_file: Some(location.file.clone()),
                resolved_line: Some(location.line),
            });
        }
        let resolved = resolve_kotlin_breakpoint(
            &self.repo_root,
            &self.generated_kotlin_jar,
            Utf8Path::new(&location.file),
            location.line,
        )?;
        for entry in &resolved {
            let _ = self.run_command(&format!("stop at {}:{}", entry.class_name, entry.line))?;
        }
        self.breakpoints.insert(key.clone(), resolved);
        Ok(DebugBreakpoint {
            debugger: DebuggerKind::Jvm,
            file: location.file.clone(),
            line: location.line,
            id: key,
            resolved_file: Some(location.file.clone()),
            resolved_line: Some(location.line),
        })
    }

    /// # Errors
    ///
    /// Returns an error if the breakpoint could not be removed.
    pub fn clear_breakpoint(&mut self, location: &DebugSourceLocation) -> AtomResult<()> {
        let key = breakpoint_key(location);
        let resolved = self.breakpoints.remove(&key).ok_or_else(|| {
            AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                format!("no active JDB breakpoint matches {}", key),
            )
        })?;
        for entry in resolved {
            let _ = self.run_command(&format!("clear {}:{}", entry.class_name, entry.line))?;
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the JVM does not stop before the timeout expires.
    pub fn wait_for_stop(&mut self, timeout_ms: Option<u64>) -> AtomResult<DebugStop> {
        if !self.running {
            return self.current_stop.clone().ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    "JDB session is already suspended but no stop reason is available",
                )
            });
        }
        let timeout = timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_WAIT_TIMEOUT);
        let (output, prompt) = self.read_until_prompt(timeout)?;
        self.running = false;
        let stop = parse_stop_output(&output, &self.breakpoints)
            .or_else(|| match prompt {
                JdbPrompt::Stopped(thread) => Some(DebugStop {
                    debugger: DebuggerKind::Jvm,
                    reason: "paused".to_owned(),
                    description: None,
                    thread_id: Some(thread.clone()),
                    thread_name: Some(thread),
                    breakpoint_id: None,
                    file: None,
                    line: None,
                }),
                JdbPrompt::Running => None,
            })
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("JDB did not report a stop event:\n{output}"),
                )
            })?;
        self.current_stop = Some(stop.clone());
        Ok(stop)
    }

    /// # Errors
    ///
    /// Returns an error if thread metadata could not be captured.
    pub fn threads(&mut self) -> AtomResult<Vec<DebugThread>> {
        self.ensure_paused("inspect threads")?;
        let output = self.run_command("threads")?;
        Ok(parse_threads(&output))
    }

    /// # Errors
    ///
    /// Returns an error if backtrace metadata could not be captured.
    pub fn backtrace(&mut self, thread_id: Option<&str>) -> AtomResult<DebugBacktrace> {
        self.ensure_paused("inspect backtraces")?;
        let threads = self.threads()?;
        let thread = thread_id
            .and_then(|requested| {
                threads.iter().find(|thread| {
                    thread.id == requested || thread.name.as_deref() == Some(requested)
                })
            })
            .or_else(|| threads.iter().find(|thread| thread.selected))
            .or_else(|| threads.first())
            .ok_or_else(|| {
                AtomError::new(
                    AtomErrorCode::AutomationUnavailable,
                    "JDB did not return any threads to backtrace",
                )
            })?;
        let thread_name = thread.name.as_deref().unwrap_or(thread.id.as_str());
        let attempts = backtrace_commands(thread);
        let mut outputs = Vec::new();
        for command in attempts {
            let output = self.run_command(&command)?;
            match parse_selected_backtrace(&output, Some(thread_name)) {
                Ok(backtrace) => return Ok(backtrace),
                Err(_) => outputs.push((command, output)),
            }
        }

        let details = outputs
            .into_iter()
            .map(|(command, output)| format!("{command}:\n{}", output.trim()))
            .collect::<Vec<_>>()
            .join("\n\n");
        Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            format!("JDB did not return a backtrace for the selected thread:\n{details}"),
        ))
    }

    /// # Errors
    ///
    /// Returns an error if the JVM could not be suspended.
    pub fn pause(&mut self) -> AtomResult<DebugStop> {
        if !self.running
            && let Some(stop) = self.current_stop.clone()
        {
            return Ok(stop);
        }
        let output = self.run_command("suspend")?;
        self.running = false;
        let selected_thread = self
            .run_command("threads")
            .ok()
            .map(|threads_output| parse_threads(&threads_output))
            .and_then(|threads| threads.into_iter().find(|thread| thread.selected));
        let stop = DebugStop {
            debugger: DebuggerKind::Jvm,
            reason: "paused".to_owned(),
            description: (!output.trim().is_empty()).then(|| output.trim().to_owned()),
            thread_id: selected_thread.as_ref().map(|thread| thread.id.clone()),
            thread_name: selected_thread.and_then(|thread| thread.name),
            breakpoint_id: None,
            file: None,
            line: None,
        };
        self.current_stop = Some(stop.clone());
        Ok(stop)
    }

    /// # Errors
    ///
    /// Returns an error if the JVM could not be resumed.
    pub fn resume(&mut self) -> AtomResult<()> {
        if self.running {
            return Ok(());
        }
        let (output, prompt) = self.run_command_with_prompt("cont")?;
        if let Some(stop) = parse_stop_output(&output, &self.breakpoints) {
            self.running = false;
            self.current_stop = Some(stop);
        } else {
            match prompt {
                JdbPrompt::Running => {
                    self.running = true;
                    self.current_stop = None;
                }
                JdbPrompt::Stopped(thread) => {
                    self.running = false;
                    self.current_stop = Some(DebugStop {
                        debugger: DebuggerKind::Jvm,
                        reason: "paused".to_owned(),
                        description: (!output.trim().is_empty()).then(|| output.trim().to_owned()),
                        thread_id: Some(thread.clone()),
                        thread_name: Some(thread),
                        breakpoint_id: None,
                        file: None,
                        line: None,
                    });
                }
            }
        }
        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error if the JDB session could not be terminated cleanly.
    pub fn shutdown(&mut self) -> AtomResult<()> {
        let _ = self.send_line("exit");
        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match self.child.try_wait().map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to poll JDB during shutdown: {error}"),
                )
            })? {
                Some(_) => break,
                None if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    break;
                }
                None => thread::sleep(Duration::from_millis(100)),
            }
        }
        let _ = run_command(
            &self.repo_root,
            "adb",
            &[
                "-s".to_owned(),
                self.serial.clone(),
                "forward".to_owned(),
                "--remove".to_owned(),
                format!("tcp:{}", self.local_port),
            ],
        );
        Ok(())
    }

    fn ensure_paused(&self, action: &str) -> AtomResult<()> {
        if self.running {
            Err(AtomError::new(
                AtomErrorCode::AutomationUnavailable,
                format!("JDB session must be suspended before it can {action}"),
            ))
        } else {
            Ok(())
        }
    }

    fn run_command(&mut self, command: &str) -> AtomResult<String> {
        self.run_command_with_prompt(command)
            .map(|(output, _)| output)
    }

    fn run_command_with_prompt(&mut self, command: &str) -> AtomResult<(String, JdbPrompt)> {
        self.send_line(command)?;
        let (output, prompt) = self.read_until_prompt(COMMAND_TIMEOUT)?;
        Ok((normalize_command_output(&output, command), prompt))
    }

    fn send_line(&mut self, command: &str) -> AtomResult<()> {
        self.stdin
            .write_all(command.as_bytes())
            .and_then(|()| self.stdin.write_all(b"\n"))
            .and_then(|()| self.stdin.flush())
            .map_err(|error| {
                AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!("failed to write JDB command: {error}"),
                )
            })
    }

    fn read_until_prompt(&mut self, timeout: Duration) -> AtomResult<(String, JdbPrompt)> {
        let deadline = Instant::now() + timeout;
        let mut buffer = Vec::new();
        loop {
            if Instant::now() >= deadline {
                return Err(AtomError::new(
                    AtomErrorCode::ExternalToolFailed,
                    format!(
                        "timed out while waiting for JDB output:\n{}",
                        String::from_utf8_lossy(&buffer)
                    ),
                ));
            }
            match self.output_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(byte) => {
                    buffer.push(byte);
                    let text = String::from_utf8_lossy(&buffer).replace('\r', "");
                    if let Some(prompt) = detect_prompt(&text) {
                        let trimmed = text[..text.len() - prompt_len(&prompt)].to_owned();
                        return Ok((trimmed, prompt));
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if let Some(status) = self.child.try_wait().map_err(|error| {
                        AtomError::new(
                            AtomErrorCode::ExternalToolFailed,
                            format!("failed to poll JDB process: {error}"),
                        )
                    })? {
                        return Err(AtomError::new(
                            AtomErrorCode::ExternalToolFailed,
                            format!(
                                "JDB exited unexpectedly with status {status}:\n{}",
                                String::from_utf8_lossy(&buffer)
                            ),
                        ));
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(AtomError::new(
                        AtomErrorCode::ExternalToolFailed,
                        "JDB output stream closed unexpectedly",
                    ));
                }
            }
        }
    }
}

fn breakpoint_key(location: &DebugSourceLocation) -> String {
    format!("{}:{}", location.file, location.line)
}

fn normalize_command_output(output: &str, command: &str) -> String {
    let mut lines = output.lines().collect::<Vec<_>>();
    if let Some(first) = lines.first().copied() {
        let first = strip_prompt_prefix(first);
        if first == command {
            lines.remove(0);
        } else {
            lines[0] = first;
        }
    }
    lines.join("\n").trim().to_owned()
}

fn strip_prompt_prefix(line: &str) -> &str {
    if let Some(stripped) = line.strip_prefix("> ") {
        return stripped;
    }
    if let Some((_, stripped)) = line.split_once("> ") {
        return stripped;
    }
    let line = line.trim_start_matches('\r');
    if let Some(index) = line.find('[')
        && index > 0
        && line[..index]
            .chars()
            .any(|character| !character.is_whitespace())
        && let Some(end_rel) = line[index + 1..].find(']')
    {
        let end = index + 1 + end_rel;
        let depth = &line[index + 1..end];
        let after = &line[end + 1..];
        if !depth.is_empty()
            && depth.chars().all(|character| character.is_ascii_digit())
            && after.starts_with(char::is_whitespace)
        {
            return after.trim_start();
        }
    }
    line
}

fn detect_prompt(text: &str) -> Option<JdbPrompt> {
    if text.ends_with("> ") {
        return Some(JdbPrompt::Running);
    }
    let tail = text.rsplit('\n').next().unwrap_or(text);
    let tail = tail.trim_start_matches('\r');
    let bracket = tail.rfind('[')?;
    if !tail.ends_with("] ") {
        return None;
    }
    let thread = tail[..bracket].trim();
    let depth = tail[bracket + 1..tail.len() - 2].trim();
    depth
        .chars()
        .all(|character| character.is_ascii_digit())
        .then(|| JdbPrompt::Stopped(thread.to_owned()))
}

fn prompt_len(prompt: &JdbPrompt) -> usize {
    match prompt {
        JdbPrompt::Running => 2,
        JdbPrompt::Stopped(thread) => thread.len() + 4,
    }
}

fn parse_stop_output(
    output: &str,
    breakpoints: &BTreeMap<String, Vec<ResolvedJvmBreakpoint>>,
) -> Option<DebugStop> {
    let line = output
        .lines()
        .find(|line| line.contains("Breakpoint hit:"))?;
    let thread_name = line
        .split("thread=")
        .nth(1)
        .and_then(|value| value.split('"').next())
        .map(ToOwned::to_owned);
    let line_number = line
        .split("line=")
        .nth(1)
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u32>().ok());
    let (file, breakpoint_id) = line_number
        .and_then(|line_number| {
            breakpoints.iter().find_map(|(id, resolved)| {
                resolved
                    .iter()
                    .any(|entry| entry.line == line_number)
                    .then(|| {
                        let file = id.rsplit_once(':').map(|(file, _)| file.to_owned());
                        (file, Some(id.clone()))
                    })
            })
        })
        .unwrap_or((None, None));
    Some(DebugStop {
        debugger: DebuggerKind::Jvm,
        reason: "breakpoint".to_owned(),
        description: Some(output.trim().to_owned()),
        thread_id: thread_name.clone(),
        thread_name,
        breakpoint_id,
        file,
        line: line_number,
    })
}

fn parse_threads(output: &str) -> Vec<DebugThread> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = strip_prompt_prefix(line).trim_start();
            if !trimmed.starts_with('(') {
                return None;
            }
            let after_paren = trimmed.split_once(')')?.1.trim_start();
            let id = after_paren
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            if id.is_empty() {
                return None;
            }
            let rest = after_paren[id.len()..].trim_start();
            let (name, state) = split_thread_name_state(rest);
            Some(DebugThread {
                debugger: DebuggerKind::Jvm,
                id,
                name: Some(name.to_owned()),
                state: Some(state.to_owned()),
                selected: state.contains("breakpoint"),
            })
        })
        .collect()
}

fn split_thread_name_state(rest: &str) -> (&str, &str) {
    for suffix in [
        "running (at breakpoint)",
        "cond. waiting",
        "not started",
        "monitor",
        "sleeping",
        "waiting",
        "running",
        "zombie",
        "unknown",
    ] {
        if let Some(name) = rest.strip_suffix(suffix) {
            return (name.trim_end(), suffix);
        }
    }
    (rest.trim(), "unknown")
}

fn backtrace_commands(thread: &DebugThread) -> Vec<String> {
    let mut commands = Vec::new();
    commands.push(format!("where {}", thread.id));
    if let Some(name) = thread.name.as_deref()
        && !name.is_empty()
        && !name.contains(char::is_whitespace)
        && name != thread.id
    {
        commands.push(format!("where {name}"));
        commands.push(format!("thread {name}"));
        commands.push("where".to_owned());
    }
    commands.push(format!("thread {}", thread.id));
    commands.push("where".to_owned());
    commands
}

fn parse_selected_backtrace(output: &str, thread_name: Option<&str>) -> AtomResult<DebugBacktrace> {
    let frames = output
        .lines()
        .filter_map(|line| parse_frame_body(strip_prompt_prefix(line).trim()))
        .enumerate()
        .map(|(index, (function, file, line))| DebugFrame {
            index: index + 1,
            function,
            module: None,
            file,
            line,
        })
        .collect::<Vec<_>>();
    if frames.is_empty() {
        return Err(AtomError::new(
            AtomErrorCode::AutomationUnavailable,
            "JDB did not return a backtrace for the selected thread",
        ));
    }
    Ok(DebugBacktrace {
        debugger: DebuggerKind::Jvm,
        thread_id: thread_name.map(ToOwned::to_owned),
        thread_name: thread_name.map(ToOwned::to_owned),
        frames,
    })
}

fn parse_frame_body(line: &str) -> Option<(String, Option<String>, Option<u32>)> {
    if line.is_empty()
        || line == "where"
        || line.starts_with("where ")
        || line.starts_with("thread ")
    {
        return None;
    }
    let body = if line.starts_with('[') {
        line.split_once(']')?.1.trim()
    } else {
        line
    };
    let (function, file, line_number) = if let Some((function, location)) = body.rsplit_once(" (") {
        let location = location.trim_end_matches(')');
        if let Some((file, line_number)) = location.rsplit_once(':') {
            (
                function.trim().to_owned(),
                Some(file.to_owned()),
                line_number.replace(',', "").parse::<u32>().ok(),
            )
        } else {
            (function.trim().to_owned(), Some(location.to_owned()), None)
        }
    } else {
        (body.to_owned(), None, None)
    };
    Some((function, file, line_number))
}

fn resolve_kotlin_breakpoint(
    repo_root: &Utf8Path,
    generated_kotlin_jar: &Utf8Path,
    source_file: &Utf8Path,
    line: u32,
) -> AtomResult<Vec<ResolvedJvmBreakpoint>> {
    let source_name = source_file.file_name().ok_or_else(|| {
        AtomError::with_path(
            AtomErrorCode::CliUsageError,
            "breakpoint source file has no file name",
            source_file.as_str(),
        )
    })?;
    let classes = list_jar_classes(repo_root, generated_kotlin_jar)?;
    let mut resolved = Vec::new();
    for class_name in classes {
        let output = capture_command(
            repo_root,
            "/usr/bin/javap",
            &[
                "-classpath".to_owned(),
                generated_kotlin_jar.as_str().to_owned(),
                "-l".to_owned(),
                class_name.clone(),
            ],
        )?;
        if !class_matches_source(&output, source_name) {
            continue;
        }
        if class_has_line(&output, line) {
            resolved.push(ResolvedJvmBreakpoint { class_name, line });
        }
    }
    if resolved.is_empty() {
        return Err(AtomError::with_path(
            AtomErrorCode::AutomationUnavailable,
            format!(
                "could not resolve Kotlin breakpoint {}:{} from generated_kotlin bytecode",
                source_file, line
            ),
            source_file.as_str(),
        ));
    }
    resolved.sort_by(|left, right| left.class_name.cmp(&right.class_name));
    resolved.dedup();
    Ok(resolved)
}

fn list_jar_classes(repo_root: &Utf8Path, jar_path: &Utf8Path) -> AtomResult<Vec<String>> {
    let output = capture_command(
        repo_root,
        "jar",
        &["tf".to_owned(), jar_path.as_str().to_owned()],
    )?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| line.ends_with(".class"))
        .filter(|line| !line.ends_with("module-info.class"))
        .map(|line| line.trim_end_matches(".class").replace('/', "."))
        .collect())
}

fn class_matches_source(javap_output: &str, source_name: &str) -> bool {
    javap_output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Compiled from "))
        .is_some_and(|value| value.trim_matches('"') == source_name)
}

fn class_has_line(javap_output: &str, line: u32) -> bool {
    javap_output.lines().any(|entry| {
        entry
            .trim()
            .strip_prefix("line ")
            .and_then(|value| value.split(':').next())
            .and_then(|value| value.trim().parse::<u32>().ok())
            == Some(line)
    })
}

fn free_tcp_port() -> AtomResult<u16> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to allocate a local TCP port: {error}"),
            )
        })?
        .local_addr()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to read allocated TCP port: {error}"),
            )
        })
        .map(|address| address.port())
}

fn run_command(repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<()> {
    let output = Command::new(tool)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "{tool} {} exited with status {}:\n{}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        ))
    }
}

fn capture_command(repo_root: &Utf8Path, tool: &str, args: &[String]) -> AtomResult<String> {
    let output = Command::new(tool)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|error| {
            AtomError::new(
                AtomErrorCode::ExternalToolFailed,
                format!("failed to invoke {tool}: {error}"),
            )
        })?;
    if !output.status.success() {
        return Err(AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!(
                "{tool} {} exited with status {}:\n{}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| {
        AtomError::new(
            AtomErrorCode::ExternalToolFailed,
            format!("{tool} returned non-UTF-8 output"),
        )
    })
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
                "failed to allocate JDB PTY: {}",
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
    use std::collections::BTreeMap;

    use super::{
        DebugThread, DebuggerKind, ResolvedJvmBreakpoint, backtrace_commands, class_has_line,
        class_matches_source, detect_prompt, normalize_command_output, parse_selected_backtrace,
        parse_stop_output, parse_threads,
    };

    #[test]
    fn prompt_detection_distinguishes_running_and_stopped() {
        assert!(matches!(
            detect_prompt("Initializing jdb ...\n> "),
            Some(super::JdbPrompt::Running)
        ));
        assert!(matches!(
            detect_prompt("Breakpoint hit\nmain[1] "),
            Some(super::JdbPrompt::Stopped(thread)) if thread == "main"
        ));
    }

    #[test]
    fn normalize_command_output_strips_prompt_and_echo() {
        assert_eq!(
            normalize_command_output("> threads\nGroup main:", "threads"),
            "Group main:"
        );
    }

    #[test]
    fn strip_prompt_prefix_handles_stopped_thread_prefixes_on_frame_lines() {
        assert_eq!(
            super::strip_prompt_prefix("main[1]   [1] JdbProbe.main (JdbProbe.java:4)"),
            "[1] JdbProbe.main (JdbProbe.java:4)"
        );
    }

    #[test]
    fn stop_output_parsing_uses_active_breakpoints() {
        let mut breakpoints = BTreeMap::new();
        breakpoints.insert(
            "/tmp/DemoSurfaceModule.kt:64".to_owned(),
            vec![ResolvedJvmBreakpoint {
                class_name: "build.atom.hello.AtomHostViewFactoryImpl".to_owned(),
                line: 64,
            }],
        );
        let stop = parse_stop_output(
            "Breakpoint hit: \"thread=main\", build.atom.hello.AtomHostViewFactoryImpl.build(), line=64 bci=1\n64             state.tapCount += 1;\n",
            &breakpoints,
        )
        .expect("stop should parse");
        assert_eq!(stop.debugger, DebuggerKind::Jvm);
        assert_eq!(stop.reason, "breakpoint");
        assert_eq!(stop.file.as_deref(), Some("/tmp/DemoSurfaceModule.kt"));
    }

    #[test]
    fn thread_parsing_extracts_ids_and_states() {
        let threads = parse_threads(
            "Group main:\n  (java.lang.Thread)1                           main                running (at breakpoint)\n",
        );
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "1");
        assert!(threads[0].selected);
    }

    #[test]
    fn thread_parsing_strips_prompt_prefixes() {
        let threads = parse_threads(
            "> Group main:\nmain[1]   (java.lang.Thread)1                           main                running\n",
        );
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].id, "1");
        assert_eq!(threads[0].name.as_deref(), Some("main"));
    }

    #[test]
    fn selected_backtrace_parsing_extracts_frames_without_thread_headers() {
        let backtrace = parse_selected_backtrace("  [1] Main.main (Main.java:5)\n", Some("main"))
            .expect("selected backtrace should parse");
        assert_eq!(backtrace.thread_name.as_deref(), Some("main"));
        assert_eq!(backtrace.frames[0].index, 1);
        assert_eq!(backtrace.frames[0].line, Some(5));
    }

    #[test]
    fn selected_backtrace_parsing_strips_stopped_thread_prefixes() {
        let backtrace = parse_selected_backtrace(
            "main[1]   [1] JdbProbe.main (JdbProbe.java:4)\n",
            Some("main"),
        )
        .expect("selected backtrace should parse after stripping the prompt prefix");
        assert_eq!(backtrace.frames[0].function, "JdbProbe.main");
        assert_eq!(backtrace.frames[0].line, Some(4));
    }

    #[test]
    fn selected_backtrace_parsing_accepts_android_jdb_where_output() {
        let backtrace = parse_selected_backtrace(
            "android.view.View.performClickInternal (View.java:8,005)\nandroid.view.View$PerformClick.run (View.java:31,229)\n",
            Some("main"),
        )
        .expect("android where output should parse");
        assert_eq!(backtrace.frames.len(), 2);
        assert_eq!(
            backtrace.frames[0].function,
            "android.view.View.performClickInternal"
        );
        assert_eq!(backtrace.frames[0].line, Some(8005));
        assert_eq!(backtrace.frames[1].index, 2);
    }

    #[test]
    fn selected_backtrace_parsing_accepts_live_android_jdb_output_shape() {
        let backtrace = parse_selected_backtrace(
            "build.atom.hello.AtomHostViewFactoryImpl$build$3$1.onClick (DemoSurfaceModule.kt:64)\nwhere main\n",
            Some("main"),
        )
        .expect("live android jdb output should parse");
        assert_eq!(
            backtrace.frames[0].function,
            "build.atom.hello.AtomHostViewFactoryImpl$build$3$1.onClick"
        );
        assert_eq!(backtrace.frames[0].line, Some(64));
    }

    #[test]
    fn backtrace_commands_try_direct_where_before_selected_thread_fallback() {
        let commands = backtrace_commands(&DebugThread {
            debugger: DebuggerKind::Jvm,
            id: "27576".to_owned(),
            name: Some("main".to_owned()),
            state: Some("running (at breakpoint)".to_owned()),
            selected: true,
        });
        assert_eq!(
            commands,
            vec![
                "where 27576",
                "where main",
                "thread main",
                "where",
                "thread 27576",
                "where",
            ]
        );
    }

    #[test]
    fn javap_source_and_line_matching_use_compiled_metadata() {
        let output = "Compiled from \"DemoSurfaceModule.kt\"\nLineNumberTable:\n  line 64: 0\n";
        assert!(class_matches_source(output, "DemoSurfaceModule.kt"));
        assert!(class_has_line(output, 64));
    }
}
