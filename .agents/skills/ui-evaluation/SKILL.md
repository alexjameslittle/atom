---
name: ui-evaluation
description:
  Drive Atom apps through framework-owned interaction and evaluation commands and collect proof
  bundles.
---

# ui-evaluation

Use Atom's interaction and evaluation commands instead of raw `idb` or `adb` calls so the public
workflow stays anchored in the framework.

## When to use

Use this skill when:

- You need to tap, long-press, swipe, drag, or type text through Atom's public CLI.
- You need one proof bundle from `atom evaluate run`.
- The hello-world demo surface is the proof surface for a framework change.

## Steps

1. Resolve a destination id first with
   `[$destination-discovery](../destination-discovery/SKILL.md)`.
2. If you need to prepare app state before ad hoc automation, prefer `atom run ios|android --detach`
   instead of holding a live log-streaming session open. Detached launch returns only after the app
   is inspectable. Use `atom stop ios|android` for cleanup only when the workflow intentionally
   launched a disposable session.
3. Run `scripts/run.sh tap`, `long-press`, `swipe`, `drag`, or `type-text` for one-step actions.
   These ad hoc commands should reuse the current foreground app state when the selected target is
   already running.
4. Run `scripts/run.sh evaluate` for a full proof bundle. The default plan is
   `../../../examples/hello-world/evaluation/demo_surface_plan.json`.

## Output

- JSON interaction result for one-step actions.
- Evaluation bundle manifest plus artifacts for full runs.

## Model vs. script split

**Script handles:** invoking `atom interact ...` and `atom evaluate run` with deterministic example
defaults.

**Model handles:** choosing the right action sequence, adjusting target ids or coordinates, and
interpreting the resulting proof bundle.
