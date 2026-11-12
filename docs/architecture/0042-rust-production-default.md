# ADR-0042: Use Rust for normal application builds

| | |
| --- | --- |
| Status | Accepted implementation direction; feature completion and human acceptance remain separate |
| Date | 2026-11-12 |
| Deciders | Product owner, with implementation by Codex for CAD-290 |

## Context

The product owner confirmed that the goal is to leave OpenCascade completely and that failures of tests requiring OpenCascade need not block this migration.
The application already selects the Rust backend when OpenCascade is absent, but default Cargo features enabled the OpenCascade build and adapter.

## Decision

Remove `occt` from the default features of the application, model, MCP and kernel crates.
Normal Cargo builds resolve the Rust geometry path without the OpenCascade build helper or CMake dependency.
Keep explicit `occt` builds available for optional comparison work during the migration.

Classify test failures by the backend they exercise.
OpenCascade-only failures do not block the Rust migration.
Actual Rust workflow, geometry, exchange and coverage failures still require resolution.
Retain independent geometry evidence without requiring repeated OpenCascade builds or prototype maintenance.

## Consequences

**Positive:** Normal builds and the default MCP process use the Rust kernel.
OpenCascade availability no longer determines whether developers exercise the replacement through the ordinary application entry point.

**Negative:** Unfinished Rust workflows now surface directly in default builds.
The legacy Native packaging path and comparison fixtures still need separate migration or classification; changing Cargo defaults does not establish package or feature completeness.

**Neutral:** The application name and canonical document remain unchanged.
The optional comparison feature is explicit.

## Alternatives considered

**Keep OpenCascade as the default until every comparison test passes.** This retains a working reference for unfinished geometry.
It keeps the production dependency active and can hide missing Rust paths, contrary to the confirmed migration direction.

## References

- [Application feature configuration](../../crates/multicad-app/Cargo.toml)
- [Backend selection](../../crates/multicad-model/src/compiler.rs)
- [Window and headless control workflows](../../crates/multicad-app/src/app_control.rs)
