# Docs

This directory is the table of contents for repository knowledge. Read top to bottom unless you
already know the area you are changing.

## Start Here

- [getting-started.md](getting-started.md): install Atom, create a project, and run it on a
  simulator.
- [../AGENTS.md](../AGENTS.md): short repo map and working rules for agents.
- [architecture.md](architecture.md): crate boundaries, dependency direction, and metadata flow.
- [harness.md](harness.md): bootstrap, verification, hooks, and CI guardrails.

## Design Context

- [core-beliefs.md](core-beliefs.md): agent-first operating principles for this repo.
- [design-docs/README.md](design-docs/README.md): stable decision records for architecture choices.
- [plan.md](plan.md): roadmap and implementation sequencing.
- [../SPEC.md](../SPEC.md): normative behavior target. Keep it aligned with implementation.

## Update Rule

If you add a new cross-cutting concept, add a doc here or link to the existing source of truth from
here. Agents should not need to rediscover important conventions from code search alone.
