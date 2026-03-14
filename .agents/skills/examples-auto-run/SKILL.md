---
name: examples-auto-run
description:
  Exercise Atom's canonical hello-world consumer through prebuild and platform-specific example
  builds after framework or example changes.
---

# examples-auto-run

Use the hello-world example app as the proof surface for framework changes that should still work in
a real consumer.

## When to use

Use this skill when:

- Changes touch `examples/hello-world/`.
- Runtime, module, CNG, or deployment changes could break consumer wiring.
- You need concrete proof that the generated host tree or example builds still work.

## Steps

1. Run `scripts/run.sh smoke` for a portable dry-run prebuild.
2. Run `scripts/run.sh generated-tree` when you need the emitted host tree for inspection.
3. Run `scripts/run.sh evaluate [platform] <destination> <artifacts-dir> [plan]` when the branch
   needs a proof bundle from the hello-world demo surface.
4. Run `scripts/run.sh android` or `scripts/run.sh ios` on the appropriate host when the branch
   needs a real platform build.
5. Report skipped platform builds or evaluation runs explicitly when the host environment cannot
   support them.

## Output

- Dry-run prebuild status for the example app.
- Generated host-tree inventory when requested.
- Evaluation bundle output when an example destination is available.
- Platform build results for Android or iOS when run.

## Model vs. script split

**Script handles:** invoking the example prebuild and platform build commands with consistent
targets.

**Model handles:** deciding which modes matter for the current change, interpreting failures, and
summarizing proof artifacts.
