# AGENTS

Start here, then read [docs/README.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/README.md).

## Operating Model

- Humans steer, agents execute. Prefer direct code changes plus verification over speculative prose.
- Bazel is the source of truth for builds, tests, linting, formatting, and generated metadata.
- Do not introduce repo-local `Cargo.toml`, `Cargo.lock`, `Atom.toml`, or `Atom.module.toml`.
- App configuration lives in `atom_app(...)`.
- Module configuration lives in `atom_module(...)` and `atom_native_module(...)`.

## Verify First

- Bootstrap: `./scripts/bootstrap.sh`
- Format: `mise run fmt`
- Verify: `mise run verify`
- Smoke prebuild: `bazel run //:atom -- prebuild --target //examples/hello-world/apps/hello_atom:hello_atom --dry-run`

Local and CI verification must stay aligned. If you add a new required check, add it to [`scripts/verify.sh`](/Users/alexlittle/conductor/workspaces/atom/tehran/scripts/verify.sh).

## Architecture Boundaries

The intended dependency flow is:

`atom-ffi` -> `atom-manifest` -> `atom-modules` -> `atom-cng` -> `atom-cli`

`atom-runtime` stays separate from CLI/CNG orchestration code.

Crate responsibilities:

- `atom-ffi`: stable error types, FlatBuffer error payloads, low-level ABI types.
- `atom-manifest`: app metadata loading and validation from Bazel-generated JSON.
- `atom-modules`: module metadata loading, validation, and dependency ordering.
- `atom-cng`: deterministic generation planning and emitted host tree writes.
- `atom-cli`: thin Bazel-facing command wrapper.
- `atom-runtime`: runtime primitives and host-facing execution logic.

Do not add reverse dependencies across these layers without documenting the change in [`docs/architecture.md`](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/architecture.md).

## Coding Conventions

- User-facing failures should return `AtomError` / `AtomErrorCode`, not ad hoc strings.
- Keep `unsafe` code narrow, documented, and isolated to ABI boundaries.
- Keep generated outputs deterministic and repo-relative.
- When behavior changes, update docs, examples, and verification in the same change.
- Prefer repository knowledge over tribal knowledge. If a convention matters, write it down under `docs/`.

## Native Modules

- The framework generates bridge glue.
- Module authors provide platform-specific code through `ios_srcs` and `android_srcs`.
- The example consumer under [`examples/hello-world`](/Users/alexlittle/conductor/workspaces/atom/tehran/examples/hello-world) should continue to exercise both Rust-backed and native-only modules.

## Where To Change What

- Bazel rules and macros: [`bzl/atom`](/Users/alexlittle/conductor/workspaces/atom/tehran/bzl/atom)
- Verification harness: [`scripts`](/Users/alexlittle/conductor/workspaces/atom/tehran/scripts), [`.githooks`](/Users/alexlittle/conductor/workspaces/atom/tehran/.githooks), [`.github/workflows`](/Users/alexlittle/conductor/workspaces/atom/tehran/.github/workflows)
- Agent-facing docs: [`docs`](/Users/alexlittle/conductor/workspaces/atom/tehran/docs)
- Example consumer: [`examples/hello-world`](/Users/alexlittle/conductor/workspaces/atom/tehran/examples/hello-world)

## Avoid

- Adding alternative manifest layers next to Bazel metadata.
- Hand-editing generated output trees as a customization mechanism.
- Introducing hidden setup steps that are not captured by bootstrap or docs.
