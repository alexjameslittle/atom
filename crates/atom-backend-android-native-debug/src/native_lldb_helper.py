import argparse
import json
import lldb
import sys
import time


def quote(value: str) -> str:
    return '"' + value.replace("\\", "\\\\").replace('"', '\\"') + '"'


def filespec_path(filespec) -> str | None:
    if not filespec or not filespec.IsValid():
        return None
    directory = filespec.GetDirectory()
    filename = filespec.GetFilename()
    if directory and filename:
        return f"{directory}/{filename}"
    return filename or directory or None


class Session:
    def __init__(self, args):
        self.debugger = lldb.SBDebugger.Create()
        self.debugger.SetAsync(True)
        self.listener = lldb.SBListener("atom-android-native")
        self.target = self.debugger.CreateTarget(args.native_library)
        if not self.target or not self.target.IsValid():
            raise RuntimeError(f"failed to create LLDB target for {args.native_library}")
        self.handle_command(
            f"settings append target.exec-search-paths {quote(args.exec_search_path)}"
        )
        if args.source_map_prefix:
            self.handle_command(
                f"settings append target.source-map {quote(args.source_map_prefix)} {quote(args.source_map_root)}"
            )
        error = lldb.SBError()
        self.process = self.target.ConnectRemote(
            self.listener,
            f"connect://127.0.0.1:{args.connect_port}",
            "gdb-remote",
            error,
        )
        if not error.Success():
            raise RuntimeError(error.GetCString() or "failed to connect remote LLDB target")

    def handle_command(self, command: str) -> None:
        interpreter = self.debugger.GetCommandInterpreter()
        result = lldb.SBCommandReturnObject()
        interpreter.HandleCommand(command, result)
        if not result.Succeeded():
            message = result.GetError() or result.GetOutput() or command
            raise RuntimeError(message.strip())

    def state(self) -> int:
        return self.process.GetState()

    def set_breakpoint(self, file: str, line: int) -> dict:
        breakpoint = self.target.BreakpointCreateByLocation(file, line)
        if not breakpoint or not breakpoint.IsValid():
            raise RuntimeError(f"failed to create native breakpoint for {file}:{line}")
        resolved_file = None
        resolved_line = None
        if breakpoint.GetNumLocations() > 0:
            location = breakpoint.GetLocationAtIndex(0)
            address = location.GetAddress()
            line_entry = address.GetLineEntry()
            if line_entry and line_entry.IsValid():
                resolved_file = filespec_path(line_entry.GetFileSpec())
                resolved_line = line_entry.GetLine()
        return {
            "debugger": "native",
            "file": file,
            "line": line,
            "id": str(breakpoint.GetID()),
            "resolved_file": resolved_file,
            "resolved_line": resolved_line,
        }

    def clear_breakpoint(self, breakpoint_id: str) -> dict:
        if not self.target.BreakpointDelete(int(breakpoint_id)):
            raise RuntimeError(f"failed to delete native breakpoint {breakpoint_id}")
        return {}

    def wait_for_stop(self, timeout_ms: int | None) -> dict:
        if self.state() == lldb.eStateStopped:
            return self.current_stop()
        timeout_ms = timeout_ms or 30000
        deadline = time.time() + (timeout_ms / 1000.0)
        event = lldb.SBEvent()
        while time.time() < deadline:
            remaining = max(0.1, deadline - time.time())
            if self.listener.WaitForEvent(min(int(remaining), 1), event):
                state = lldb.SBProcess.GetStateFromEvent(event)
                if state == lldb.eStateStopped:
                    return self.current_stop()
                if state in (lldb.eStateExited, lldb.eStateCrashed, lldb.eStateDetached):
                    raise RuntimeError(f"native process exited while waiting for a stop (state={state})")
            if self.state() == lldb.eStateStopped:
                return self.current_stop()
        raise RuntimeError("timed out while waiting for a native stop")

    def threads(self) -> list[dict]:
        return [self.thread_payload(self.process.GetThreadAtIndex(i)) for i in range(self.process.GetNumThreads())]

    def backtrace(self, thread_id: str | None) -> dict:
        thread = self.select_thread(thread_id)
        return {
            "debugger": "native",
            "thread_id": str(thread.GetThreadID()),
            "thread_name": thread.GetName() or thread.GetQueueName(),
            "frames": [self.frame_payload(thread.GetFrameAtIndex(i), i) for i in range(thread.GetNumFrames())],
        }

    def pause(self) -> dict:
        if self.state() != lldb.eStateStopped:
            error = self.process.Stop()
            if not error.Success():
                raise RuntimeError(error.GetCString() or "failed to stop native process")
            return self.wait_for_stop(10000)
        return self.current_stop()

    def resume(self) -> dict:
        if self.state() == lldb.eStateStopped:
            error = self.process.Continue()
            if not error.Success():
                raise RuntimeError(error.GetCString() or "failed to continue native process")
        return {}

    def shutdown(self) -> dict:
        if self.state() == lldb.eStateStopped:
            self.process.Detach()
        else:
            self.process.Detach()
        self.debugger.Destroy(self.debugger)
        return {}

    def current_stop(self) -> dict:
        thread = self.process.GetSelectedThread()
        if not thread or not thread.IsValid():
            for index in range(self.process.GetNumThreads()):
                candidate = self.process.GetThreadAtIndex(index)
                if candidate.GetStopReason() != lldb.eStopReasonNone:
                    thread = candidate
                    break
        reason = thread.GetStopReason() if thread and thread.IsValid() else lldb.eStopReasonNone
        description = thread.GetStopDescription(256) if thread and thread.IsValid() else None
        file_path = None
        line_number = None
        if thread and thread.IsValid() and thread.GetNumFrames() > 0:
            frame = thread.GetFrameAtIndex(0)
            line_entry = frame.GetLineEntry()
            if line_entry and line_entry.IsValid():
                file_path = filespec_path(line_entry.GetFileSpec())
                line_number = line_entry.GetLine()
        breakpoint_id = None
        if reason == lldb.eStopReasonBreakpoint and thread.GetStopReasonDataCount() >= 2:
            breakpoint_id = str(thread.GetStopReasonDataAtIndex(0))
        return {
            "debugger": "native",
            "reason": self.reason_name(reason),
            "description": description,
            "thread_id": str(thread.GetThreadID()) if thread and thread.IsValid() else None,
            "thread_name": (thread.GetName() or thread.GetQueueName()) if thread and thread.IsValid() else None,
            "breakpoint_id": breakpoint_id,
            "file": file_path,
            "line": line_number,
        }

    def reason_name(self, reason: int) -> str:
        if reason == lldb.eStopReasonBreakpoint:
            return "breakpoint"
        if reason == lldb.eStopReasonSignal:
            return "paused"
        if reason == lldb.eStopReasonException:
            return "exception"
        if reason == lldb.eStopReasonTrace:
            return "trace"
        if reason == lldb.eStopReasonWatchpoint:
            return "watchpoint"
        if self.state() == lldb.eStateStopped:
            return "paused"
        return "unknown"

    def thread_payload(self, thread) -> dict:
        return {
            "debugger": "native",
            "id": str(thread.GetThreadID()),
            "name": thread.GetName() or thread.GetQueueName(),
            "state": self.reason_name(thread.GetStopReason()) if self.state() == lldb.eStateStopped else "running",
            "selected": self.process.GetSelectedThread().GetThreadID() == thread.GetThreadID(),
        }

    def frame_payload(self, frame, index: int) -> dict:
        line_entry = frame.GetLineEntry()
        file_path = filespec_path(line_entry.GetFileSpec()) if line_entry and line_entry.IsValid() else None
        line_number = line_entry.GetLine() if line_entry and line_entry.IsValid() else None
        module = frame.GetModule().GetFileSpec().GetFilename() if frame.GetModule().IsValid() else None
        return {
            "index": index,
            "function": frame.GetFunctionName() or frame.GetDisplayFunctionName() or frame.GetSymbol().GetName() or "<unknown>",
            "module": module,
            "file": file_path,
            "line": line_number,
        }

    def select_thread(self, thread_id: str | None):
        if not thread_id:
            return self.process.GetSelectedThread()
        for index in range(self.process.GetNumThreads()):
            thread = self.process.GetThreadAtIndex(index)
            if str(thread.GetThreadID()) == thread_id:
                return thread
        raise RuntimeError(f"native thread {thread_id} was not found")


def write_response(ok: bool, value=None, message: str | None = None) -> None:
    payload = {"ok": ok}
    if value is not None:
        payload["value"] = value
    if message is not None:
        payload["message"] = message
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--connect-port", required=True, type=int)
    parser.add_argument("--native-library", required=True)
    parser.add_argument("--exec-search-path", required=True)
    parser.add_argument("--source-map-prefix", required=True)
    parser.add_argument("--source-map-root", required=True)
    args = parser.parse_args()
    try:
        session = Session(args)
        sys.stdout.write(json.dumps({"ready": True}) + "\n")
        sys.stdout.flush()
    except Exception as error:
        write_response(False, message=str(error))
        return 1

    for line in sys.stdin:
        if not line.strip():
            continue
        try:
            request = json.loads(line)
            command = request["command"]
            if command == "set_breakpoint":
                write_response(True, session.set_breakpoint(request["file"], request["line"]))
            elif command == "clear_breakpoint":
                write_response(True, session.clear_breakpoint(request["breakpoint_id"]))
            elif command == "wait_for_stop":
                write_response(True, session.wait_for_stop(request.get("timeout_ms")))
            elif command == "threads":
                write_response(True, session.threads())
            elif command == "backtrace":
                write_response(True, session.backtrace(request.get("thread_id")))
            elif command == "pause":
                write_response(True, session.pause())
            elif command == "resume":
                write_response(True, session.resume())
            elif command == "shutdown":
                write_response(True, session.shutdown())
                return 0
            else:
                write_response(False, message=f"unknown command: {command}")
        except Exception as error:
            write_response(False, message=str(error))
    return 0


if __name__ == "__main__":
    sys.exit(main())
