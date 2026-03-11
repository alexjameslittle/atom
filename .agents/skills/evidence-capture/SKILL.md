---
name: evidence-capture
description:
  Capture logs, screenshots, video, and UI snapshots through Atom's framework-owned evidence
  commands.
---

# evidence-capture

Prefer Atom's evidence commands over ad hoc host tooling so proof artifacts stay consistent across
agents and humans.

## When to use

Use this skill when:

- A change needs a screenshot, video, or log artifact from a running Atom app.
- You need a machine-readable UI snapshot for later reasoning.
- You want artifacts written to explicit caller-selected paths instead of tool defaults.

## Steps

1. Resolve a destination id first with
   `[$destination-discovery](../destination-discovery/SKILL.md)`.
2. If the app is not already running and you want to preserve state across multiple captures, prefer
   `atom run ios|android --detach` instead of keeping a log-streaming session open.
3. Run `scripts/run.sh screenshot`, `logs`, `video`, or `inspect-ui` with the chosen destination.
   These commands should reuse the current foreground app state when the selected target is already
   running. Prefer the focused log output these commands produce over raw device-wide syslog or
   logcat when you are validating app behavior. On iOS simulators, screenshot capture may fall back
   to `simctl io screenshot` when `idb` cannot encode an image. On iOS, prefer `.mov` output paths
   for video capture; on Android, prefer `.mp4`.
4. Use `atom stop ios|android` for cleanup only when the workflow explicitly launched a disposable
   detached session.
5. Keep the output paths stable so the evidence can be referenced in follow-on analysis.

## Output

- Screenshot, log, video, or UI snapshot artifact at the requested path.

## Model vs. script split

**Script handles:** invoking `atom evidence ...` and `atom inspect ui` with the required flags.

**Model handles:** deciding which artifact type is needed, choosing durations, and interpreting the
captured output.
