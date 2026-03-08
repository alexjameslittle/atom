# Docs

This directory is the table of contents for repository knowledge. Read top to bottom unless you
already know the area you are changing.

## Start Here

- [../AGENTS.md](/Users/alexlittle/conductor/workspaces/atom/tehran/AGENTS.md): short repo map and
  working rules for agents.
- [architecture.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/architecture.md): crate
  boundaries, dependency direction, and metadata flow.
- [harness.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/harness.md): bootstrap,
  verification, hooks, and CI guardrails.

## Design Context

- [core-beliefs.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/core-beliefs.md):
  agent-first operating principles for this repo.
- [design-docs/README.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/design-docs/README.md):
  stable decision records for architecture choices.
- [plan.md](/Users/alexlittle/conductor/workspaces/atom/tehran/docs/plan.md): roadmap and
  implementation sequencing.
- [../SPEC.md](/Users/alexlittle/conductor/workspaces/atom/tehran/SPEC.md): normative behavior
  target. Keep it aligned with implementation.

## Update Rule

If you add a new cross-cutting concept, add a doc here or link to the existing source of truth from
here. Agents should not need to rediscover important conventions from code search alone.
