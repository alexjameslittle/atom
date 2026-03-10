---
name: destination-discovery
description:
  Discover Atom destination ids before evidence capture, UI interaction, or evaluation work.
---

# destination-discovery

Use Atom's destination model instead of raw `idb` or `adb` output when choosing a target for runtime
proof.

## When to use

Use this skill when:

- A workflow needs one reusable `destination` id across multiple Atom commands.
- You need to inspect available iOS or Android targets before running evidence or evaluation steps.
- You need machine-readable destination data to reason about capabilities.

## Steps

1. Run `scripts/run.sh all-json` for the full destination inventory.
2. Run `scripts/run.sh ios-json` or `scripts/run.sh android-json` when only one platform matters.
3. Pick an available destination id whose capabilities cover the intended evidence or evaluation
   workflow.

## Output

- Human-readable or JSON destination inventory.
- One stable `destination` id for follow-on Atom commands.

## Model vs. script split

**Script handles:** invoking the framework-owned destination listing commands.

**Model handles:** filtering by capability, choosing the right destination, and explaining tradeoffs
between simulator, emulator, and device targets.
