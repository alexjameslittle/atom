# AGENTS

Atom is a Rust-first mobile application framework. App logic, runtime plugins, and modules are
authored in Rust. Continuous Native Generation (CNG) derives Swift/Kotlin host code from Bazel
metadata. The CLI orchestrates Bazel builds, device deployment, and code generation. There is no UI
renderer — navigation, persistence, and analytics are plugins, not kernel features.

Start here, then read [docs/README.md](docs/README.md).

## Operating Model

- Humans steer, agents execute. Prefer direct code changes plus verification over speculative prose.
- Bazel is the source of truth for builds, tests, linting, formatting, and generated metadata.
- Do not introduce repo-local `Cargo.toml`, `Cargo.lock`, `Atom.toml`, or `Atom.module.toml`.
- App configuration lives in `atom_app(...)`.
- Module configuration lives in `atom_module(...)` and `atom_native_module(...)`.

## Skills

Skills live in `.agents/skills/`. Each skill has a `SKILL.md` manifest and optional `scripts/` that
handle deterministic work. Read a skill's `SKILL.md` before using it.

### Mandatory skill triggers

| Skill                                                              | Trigger                                                                                                                                        |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| [code-verification](.agents/skills/code-verification/SKILL.md)     | **Any change** to `crates/`, `bzl/`, `examples/`, `scripts/`, `.github/workflows/`, `MODULE.bazel`, or `.bazelversion`. Run before committing. |
| [docs-sync](.agents/skills/docs-sync/SKILL.md)                     | Changes to public API surface, crate additions/removals, architecture boundary changes, or plan/spec updates.                                  |
| [architecture-review](.agents/skills/architecture-review/SKILL.md) | New cross-crate dependencies, new crates, or types moved across crate boundaries.                                                              |
| [prebuild-validation](.agents/skills/prebuild-validation/SKILL.md) | Changes to `crates/atom-cng/`, `templates/`, `bzl/atom/`, or module metadata attributes.                                                       |
| [spec-sync](.agents/skills/spec-sync/SKILL.md)                     | Behavior changes, new error codes, lifecycle states, metadata fields, or release candidates.                                                   |

### How skills work

Skills follow a **model vs. script split**:

- **Scripts** handle deterministic shell work: running commands, collecting output, hashing files.
- **Model** handles interpretation: judging failures, suggesting fixes, drafting updates.

Skills use **progressive disclosure**: metadata loads first, full SKILL.md loads when the skill is
selected, scripts run only when needed. This prevents bloating agent context.

## Verify First

- Bootstrap: `./scripts/bootstrap.sh`
- Format: `mise run fmt`
- Verify: `mise run verify`
- Smoke prebuild:
  `bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`

Local and CI verification must stay aligned. If you add a new required check, add it to
[`scripts/verify.sh`](scripts/verify.sh).

## Architecture Boundaries

The intended dependency flow is:

`atom-ffi` -> `atom-manifest` -> `atom-modules` -> `atom-cng` -> `atom-deploy` -> `atom-cli`

`atom-runtime` stays separate from CLI/CNG orchestration code.

Crate responsibilities:

- `atom-ffi`: stable error types, FlatBuffer error payloads, low-level ABI types.
- `atom-manifest`: app metadata loading and validation from Bazel-generated JSON.
- `atom-modules`: module metadata loading, validation, and dependency ordering.
- `atom-cng`: deterministic generation planning and emitted host tree writes.
- `atom-deploy`: device discovery, platform deployment, and external tool orchestration.
- `atom-cli`: thin CLI command dispatch and workspace resolution.
- `atom-runtime`: runtime primitives and host-facing execution logic.

Do not add reverse dependencies across these layers without documenting the change in
[`docs/architecture.md`](docs/architecture.md).

## Coding Conventions

- User-facing failures should return `AtomError` / `AtomErrorCode`, not ad hoc strings.
- Keep `unsafe` code narrow, documented, and isolated to ABI boundaries.
- Keep generated outputs deterministic and repo-relative.
- When behavior changes, update docs, examples, and verification in the same change.
- Prefer repository knowledge over tribal knowledge. If a convention matters, write it down under
  `docs/`.

## Native Modules

- The framework generates bridge glue.
- Module authors provide platform-specific code through `ios_srcs` and `android_srcs`.
- The example consumer under [`examples/hello-world`](examples/hello-world) should continue to
  exercise both Rust-backed and native-only modules.

## Where To Change What

- Bazel rules and macros: [`bzl/atom`](bzl/atom)
- Verification harness: [`scripts`](scripts), [`.githooks`](.githooks),
  [`.github/workflows`](.github/workflows)
- Agent skills: [`.agents/skills`](.agents/skills)
- Agent-facing docs: [`docs`](docs)
- Example consumer: [`examples/hello-world`](examples/hello-world)

## Avoid

- Adding alternative manifest layers next to Bazel metadata.
- Hand-editing generated output trees as a customization mechanism.
- Introducing hidden setup steps that are not captured by bootstrap or docs.
- Skipping mandatory skills when their trigger conditions are met.
