<!-- Kind: decision; Audience: native architecture maintainers; Purpose: record equipment host and drawing source ownership; Non-goals: standards compliance or human acceptance. -->
# ADR 0028: Hosted equipment and linked plan symbols

Status: Proposed — 2026-11-01. Work item: [CAD-203](https://plans.tokenet.de/CAD/203).

## Context

Apartment equipment must follow floor, wall and ceiling edits while its physical geometry, plan footprint and labels retain one identity. A copied world pose loses mounting-height meaning. Independent 2D symbols drift after a model edit. Electrical devices also need a readable plan mark when the plan cut misses their physical body.

## Decision

Retain equipment as pinned dynamic symbols with typed equipment metadata and canonical width/depth/height parameters. Add an absolute equipment attachment placement driver referencing the host floor and optional wall. Its dependencies exclude the owning Instance and symbol record. Use explicit host-local coordinates and mounting datum semantics; unresolved hosts require repair.

Capture a true-size authored footprint and an optional separately sized paper mark in each definition. Substitute them only in aligned top projections, using the occurrence pose and current private parameters. Store category filtering on the DrawingView. Use owned generic v1 symbols with explicit Type F terminology; standards compliance requires separately licensed and verified assets.

Reuse existing service-envelope geometry for optional door/access and service boxes. Keep physical geometry, overlay visibility and geometric clearance checking distinct. Move/Copy retains host-local intent and clones private semantic source and owned envelopes.

## Consequences

Projects require explicit equipment and attachment capabilities. Host topology changes beyond the supported path contract surface repair errors. Generic geometry and access boxes cannot certify manufacturer fit or installation rules. Native parameter, history, output and graphical checks are required before the implementation is submitted for review. Human acceptance remains separate.
