import argparse
import json
import subprocess
import sys
import time
from typing import Dict, Optional

import lldb


class HelperError(Exception):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message


class DebugSession:
    def __init__(
        self,
        repo_root: str,
        destination_id: str,
        bundle_id: str,
        dsym_bundle: Optional[str],
    ) -> None:
        self.repo_root = repo_root
        self.destination_id = destination_id
        self.bundle_id = bundle_id
        self.dsym_bundle = dsym_bundle
        self.debugger = None
        self.attached = False

    def handle(self, request: Dict[str, object]) -> Dict[str, object]:
        kind = request.get("kind")
        if kind == "attach":
            return self.attach()
        if kind == "inspect_state":
            return {"kind": "state", "state": self.state_value()}
        if kind == "wait_for_stop":
            return self.wait_for_stop(request["timeout_ms"])
        if kind == "pause":
            return self.pause()
        if kind == "resume":
            return self.resume()
        if kind == "list_threads":
            return self.list_threads()
        if kind == "list_frames":
            return self.list_frames(request.get("thread_id"))
        raise HelperError("automation_unavailable", f"unsupported debugger request: {kind}")

    def attach(self) -> dict:
        if self.attached:
            return {"kind": "attached", "state": self.state_value()}

        self.cleanup_debugger()
        self.debugger = lldb.SBDebugger.Create()
        self.debugger.SkipLLDBInitFiles(True)
        self.debugger.SetAsync(True)

        process_id = self.wait_for_app_process(timeout_seconds=5.0)
        self.run_command(f"process attach --pid {process_id}")
        self.attached = True
        self.wait_for_state("stopped", timeout_seconds=2.0)
        return {"kind": "attached", "state": self.state_value()}

    def wait_for_stop(self, timeout_ms: int) -> dict:
        self.ensure_attached()
        self.wait_for_state("stopped", timeout_seconds=timeout_ms / 1000.0)
        return {"kind": "stopped", "state": "stopped"}

    def pause(self) -> dict:
        self.ensure_attached()
        if self.state_value() == "stopped":
            return {"kind": "paused"}
        error = self.process().Stop()
        if error.Fail():
            raise HelperError("external_tool_failed", error.GetCString() or "failed to interrupt the iOS process")
        self.wait_for_state("stopped", timeout_seconds=5.0)
        return {"kind": "paused"}

    def resume(self) -> dict:
        self.ensure_attached()
        if self.state_value() == "running":
            return {"kind": "resumed"}
        error = self.process().Continue()
        if error.Fail():
            raise HelperError("external_tool_failed", error.GetCString() or "failed to continue the iOS process")
        self.wait_for_state("running", timeout_seconds=2.0)
        return {"kind": "resumed"}

    def list_threads(self) -> dict:
        process = self.require_stopped_process()
        selected = process.GetSelectedThread()
        selected_id = str(selected.GetThreadID()) if selected.IsValid() else None
        threads = []
        for index in range(process.GetNumThreads()):
            thread = process.GetThreadAtIndex(index)
            threads.append(
                {
                    "id": str(thread.GetThreadID()),
                    "name": thread.GetName() or None,
                    "selected": str(thread.GetThreadID()) == selected_id,
                }
            )
        return {"kind": "threads", "threads": threads}

    def list_frames(self, thread_id: Optional[str]) -> dict:
        thread = self.resolve_thread(thread_id)
        frames = []
        for index in range(thread.GetNumFrames()):
            frame = thread.GetFrameAtIndex(index)
            line_entry = frame.GetLineEntry()
            source_path = None
            line = None
            column = None
            if line_entry.IsValid():
                file_spec = line_entry.GetFileSpec()
                directory = file_spec.GetDirectory()
                filename = file_spec.GetFilename()
                if directory and filename:
                    source_path = f"{directory}/{filename}"
                elif filename:
                    source_path = filename
                if line_entry.GetLine() > 0:
                    line = line_entry.GetLine()
                if line_entry.GetColumn() > 0:
                    column = line_entry.GetColumn()
            function = frame.GetFunctionName()
            if not function:
                symbol = frame.GetPCAddress().GetSymbol()
                if symbol.IsValid():
                    function = symbol.GetName()
            frames.append(
                {
                    "index": index,
                    "function": function or "<unknown>",
                    "source_path": source_path,
                    "line": line,
                    "column": column,
                }
            )
        return {"kind": "frames", "thread_id": str(thread.GetThreadID()), "frames": frames}

    def resolve_thread(self, thread_id: Optional[str]):
        process = self.require_stopped_process()
        if thread_id is None:
            thread = process.GetSelectedThread()
            if thread.IsValid():
                return thread
            if process.GetNumThreads() > 0:
                return process.GetThreadAtIndex(0)
            raise HelperError("automation_unavailable", "the iOS debugger did not expose any threads")

        for index in range(process.GetNumThreads()):
            thread = process.GetThreadAtIndex(index)
            if str(thread.GetThreadID()) == thread_id:
                return thread
        raise HelperError("automation_unavailable", f"unknown iOS debugger thread id: {thread_id}")

    def require_stopped_process(self):
        process = self.process()
        if self.state_value() != "stopped":
            raise HelperError("automation_unavailable", "the iOS debugger process must be stopped before inspection")
        return process

    def ensure_attached(self) -> None:
        if not self.attached:
            raise HelperError("automation_unavailable", "the iOS debugger session is not attached")

    def wait_for_state(self, expected: str, timeout_seconds: float) -> None:
        deadline = time.time() + timeout_seconds
        while time.time() < deadline:
            if self.state_value() == expected:
                return
            self.pump_process_events(wait_seconds=1)
        if self.state_value() == expected:
            return
        raise HelperError("automation_unavailable", f"timed out waiting for the iOS debugger to become {expected}")

    def state_value(self) -> str:
        if not self.attached or self.debugger is None:
            return "unknown"
        self.pump_process_events(wait_seconds=0)
        state = self.process().GetState()
        if state in (lldb.eStateStopped, lldb.eStateCrashed, lldb.eStateSuspended):
            return "stopped"
        if state in (
            lldb.eStateRunning,
            lldb.eStateStepping,
            lldb.eStateAttaching,
            lldb.eStateLaunching,
            lldb.eStateConnected,
        ):
            return "running"
        return "unknown"

    def process(self):
        if self.debugger is None:
            raise HelperError("automation_unavailable", "the iOS debugger is not initialized")
        target = self.debugger.GetSelectedTarget()
        if not target.IsValid():
            raise HelperError("external_tool_failed", "LLDB did not select an iOS target")
        process = target.GetProcess()
        if not process.IsValid():
            raise HelperError("external_tool_failed", "LLDB did not expose an iOS process")
        return process

    def wait_for_app_process(self, timeout_seconds: float) -> int:
        deadline = time.time() + timeout_seconds
        while time.time() < deadline:
            process_id = self.lookup_app_process()
            if process_id is not None:
                return process_id
            time.sleep(0.1)
        raise HelperError(
            "automation_unavailable",
            f"timed out waiting for the iOS simulator app process for {self.bundle_id}",
        )

    def lookup_app_process(self) -> Optional[int]:
        result = subprocess.run(
            ["xcrun", "simctl", "spawn", self.destination_id, "launchctl", "list"],
            cwd=self.repo_root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        if result.returncode != 0:
            message = result.stderr.strip() or result.stdout.strip() or "failed to inspect simulator processes"
            raise HelperError("external_tool_failed", message)
        for line in result.stdout.splitlines():
            if f"UIKitApplication:{self.bundle_id}" not in line:
                continue
            columns = line.split(None, 2)
            if len(columns) < 3:
                continue
            process_id = columns[0]
            if process_id.isdigit():
                return int(process_id)
        return None

    def run_command(self, command: str) -> None:
        result = lldb.SBCommandReturnObject()
        self.debugger.GetCommandInterpreter().HandleCommand(command, result)
        if result.Succeeded():
            return
        message = result.GetError() or result.GetOutput() or f"LLDB command failed: {command}"
        raise HelperError("external_tool_failed", message.strip())

    def pump_process_events(self, wait_seconds: int) -> None:
        if self.debugger is None:
            return
        listener = self.debugger.GetListener()
        event = lldb.SBEvent()
        if wait_seconds > 0 and listener.WaitForEvent(wait_seconds, event):
            self.consume_process_event(event)
        while listener.GetNextEvent(event):
            self.consume_process_event(event)

    def consume_process_event(self, event) -> None:
        if lldb.SBProcess.EventIsProcessEvent(event):
            lldb.SBProcess.GetStateFromEvent(event)

    def cleanup(self) -> None:
        self.cleanup_debugger()

    def cleanup_debugger(self) -> None:
        if self.debugger is None:
            return
        try:
            process = self.process()
        except HelperError:
            process = None
        if process is not None:
            try:
                self.pump_process_events(wait_seconds=0)
                if process.GetState() not in (
                    lldb.eStateDetached,
                    lldb.eStateExited,
                    lldb.eStateInvalid,
                ):
                    process.Detach()
            except Exception:
                pass
        lldb.SBDebugger.Destroy(self.debugger)
        self.debugger = None
        self.attached = False


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", required=True)
    parser.add_argument("--destination-id", required=True)
    parser.add_argument("--bundle-id", required=True)
    parser.add_argument("--dsym-bundle")
    args = parser.parse_args()

    lldb.SBDebugger.Initialize()
    session = DebugSession(
        repo_root=args.repo_root,
        destination_id=args.destination_id,
        bundle_id=args.bundle_id,
        dsym_bundle=args.dsym_bundle,
    )
    try:
        for raw_line in sys.stdin:
            line = raw_line.strip()
            if not line:
                continue
            try:
                request = json.loads(line)
                reply = {"ok": True, "response": session.handle(request)}
            except HelperError as error:
                reply = {"ok": False, "code": error.code, "message": error.message}
            except Exception as error:
                reply = {"ok": False, "code": "external_tool_failed", "message": str(error)}
            sys.stdout.write(json.dumps(reply) + "\n")
            sys.stdout.flush()
    finally:
        session.cleanup()
        lldb.SBDebugger.Terminate()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
