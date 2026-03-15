# AGENTS

Atom is a Rust-first mobile application framework. App logic, runtime support libraries, and modules
are authored in Rust. Continuous Native Generation (CNG) derives Swift/Kotlin host code from Bazel
metadata. The CLI orchestrates Bazel builds, device deployment, and code generation. There is no UI
renderer; navigation, persistence, and analytics remain library concerns rather than kernel
features.

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

Skill manifests should start with YAML frontmatter that includes:

- `name`: the stable skill identifier.
- `description`: a sharp routing hint that says when the skill is useful.

Paths inside `SKILL.md` should be relative to the skill directory (`scripts/...`, `references/...`),
not repo-root wrapper paths.

### Mandatory skill triggers

| Skill                                                              | Trigger                                                                                                                                        |
| ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| [code-verification](.agents/skills/code-verification/SKILL.md)     | **Any change** to `crates/`, `bzl/`, `examples/`, `scripts/`, `.github/workflows/`, `MODULE.bazel`, or `.bazelversion`. Run before committing. |
| [docs-sync](.agents/skills/docs-sync/SKILL.md)                     | Changes to public API surface, crate additions/removals, architecture boundary changes, or plan/spec updates.                                  |
| [architecture-review](.agents/skills/architecture-review/SKILL.md) | New cross-crate dependencies, new crates, or types moved across crate boundaries.                                                              |
| [prebuild-validation](.agents/skills/prebuild-validation/SKILL.md) | Changes to `crates/atom-cng/`, `templates/`, `bzl/atom/`, or module metadata attributes.                                                       |
| [spec-sync](.agents/skills/spec-sync/SKILL.md)                     | Behavior changes, new error codes, lifecycle states, metadata fields, or release candidates.                                                   |

### Reusable workflow skills

| Skill                                                                      | Use when                                                                                             |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| [destination-discovery](.agents/skills/destination-discovery/SKILL.md)     | A task needs one stable `destination` id before launch, evidence capture, or evaluation work.        |
| [evidence-capture](.agents/skills/evidence-capture/SKILL.md)               | A task needs logs, screenshots, video, or UI snapshots from a running Atom app.                      |
| [implementation-strategy](.agents/skills/implementation-strategy/SKILL.md) | A task is multi-file, architectural, or high-risk and needs a concrete edit plan before coding.      |
| [pr-summary](.agents/skills/pr-summary/SKILL.md)                           | A branch is ready for user handoff or PR drafting and needs a clean summary plus verification block. |
| [release-review](.agents/skills/release-review/SKILL.md)                   | A release candidate or release-critical branch needs one final pass across spec, docs, and examples. |
| [test-coverage-improver](.agents/skills/test-coverage-improver/SKILL.md)   | A diff changes behavior but the right test additions are not obvious yet.                            |
| [ui-evaluation](.agents/skills/ui-evaluation/SKILL.md)                     | A task needs Atom-owned UI interaction or a proof bundle from `atom evaluate run`.                   |
| [examples-auto-run](.agents/skills/examples-auto-run/SKILL.md)             | Framework or example changes need proof through the hello-world consumer app.                        |

### How skills work

Skills follow a **model vs. script split**:

- **Scripts** handle deterministic shell work: running commands, collecting output, hashing files.
- **Model** handles interpretation: judging failures, suggesting fixes, drafting updates.

Skills use **progressive disclosure**: routing should start from frontmatter metadata, full
`SKILL.md` loads when the skill is selected, and scripts run only when needed. This prevents
bloating agent context.

## Review And Handoff

- Review for correctness, regressions, missing tests, and contract drift before style issues.
- Cite file and line evidence for findings whenever possible.
- If no findings remain, say so explicitly and note any skipped checks or residual risk.
- Before handing work back or drafting a PR, run `pr-summary` and follow
  [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md).
- Before calling a branch release-ready, run `release-review` and make any skipped checks explicit.

## Verify First

- Bootstrap: `./scripts/bootstrap.sh`
- Format: `mise run fmt`
- Verify: `mise run verify`
- Smoke prebuild:
  `mise exec -- bazelisk run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`

Local and CI verification must stay aligned. If you add a new required check, add it to
[`scripts/verify.sh`](scripts/verify.sh).

## Architecture Boundaries

The intended dependency flow is:

`atom-ffi` -> `atom-manifest` -> `atom-modules` -> `atom-backends` -> `atom-cng` -> `atom-deploy` ->
`atom-backend-{ios,android}` -> `atom-cli`

`atom-runtime` stays separate from CLI/CNG orchestration code.

Crate responsibilities:

- `atom-ffi`: stable error types, FlatBuffer error payloads, low-level ABI types, and generated
  export buffer/codec helpers.
- `atom-manifest`: app metadata loading and validation from Bazel-generated JSON.
- `atom-modules`: module metadata loading, validation, and dependency ordering.
- `atom-backends`: shared backend contracts, registries, and backend-neutral deploy/evaluate/CNG
  data types.
- `atom-cng`: deterministic generation planning and emitted host tree writes through backend
  contracts.
- `atom-deploy`: generic destination discovery, deployment, evidence capture, and UI evaluation
  orchestration through backend contracts.
- `atom-backend-ios` / `atom-backend-android`: first-party backend implementation crates linked into
  the official CLI binary.
- `atom-cli`: thin CLI command dispatch and workspace resolution.
- `atom-macros`: proc-macro ergonomics for Rust-authored module boundaries; it must stay limited to
  code generation against stable `atom-ffi` / `atom-runtime` APIs and must not absorb CNG or CLI
  policy.
- `atom-runtime`: runtime primitives and host-facing execution logic.

Generic crate invariants:

- `atom-backends`, `atom-cng`, and `atom-deploy` must stay backend-neutral.
- Do not add concrete first-party backend ids, iOS/Android-specific logic, or backend-specific tests
  to those crates.
- Put backend-specific planning, destination parsing, automation behavior, and golden-file
  assertions in `atom-backend-*` crates instead.
- If a backend crate changes behavior, keep a dedicated `rust_test` target in that crate updated in
  the same change.

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
- Reintroducing concrete `ios` / `android` branching into `atom-backends`, `atom-cng`, or
  `atom-deploy`.
- Hand-editing generated output trees as a customization mechanism.
- Introducing hidden setup steps that are not captured by bootstrap or docs.
- Skipping mandatory skills when their trigger conditions are met.
