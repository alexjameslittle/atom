# 0002: FlatBuffers For Error And Bridge Payloads

Status: accepted

## Context

Atom needs machine-readable payloads across CLI, runtime, and host boundaries. The transport should
be deterministic, compact, and language-neutral.

## Decision

- Keep a framework-owned FlatBuffers boundary for error payloads and generated bridge payloads.
- Centralize shared ABI-adjacent types in `atom-ffi`.
- Treat the ABI as a compatibility surface, not an incidental implementation detail.

## Consequences

- User-facing failures should map through `AtomError` and `AtomErrorCode`.
- Changes to payload shape should be reflected in both code and spec/docs.
- Unsafe boundary code should remain narrow and documented.
