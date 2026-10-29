# Generic CAD queue handoff, 2026-10-29

The user requested a transfer checkpoint. Stop before claiming CAD-129. Work is
committed locally on `main` in `/Users/keller/repos/_apps/multicad`; nothing was
pushed, installed or published. Production source is through `29a3afa`, symbol
verification through `f6d880d`; this handoff is a subsequent documentation commit.

## Authoritative queue state

PLAINER was checked at approximately 00:42 UTC. No CAD item is In Progress and
no execution claim remains. Engineering Review is distinct from human acceptance.

| Item | State | Evidence |
|---|---|---|
| CAD-133, Parameters | Review, revision 7 | [Parameters checkpoint](../verification/native-parameter-workflow-2026-10-28.md) |
| CAD-127, exact spline edits | Review, revision 7 | [Spline checkpoint](../verification/native-spline-editing-2026-10-28.md) |
| CAD-128, occurrence references | Review, revision 7 | [Occurrence checkpoint](../verification/native-occurrence-references-2026-10-29.md), final native check also recorded in its comment |
| CAD-70, reusable symbols | Review, revision 8 | [Symbol checkpoint](../verification/native-symbols-2026-10-29.md) and complete final Working Log |

The first eligible Next item is **CAD-129**, importing DXF/SVG as editable sketch
geometry. Its ID is `4fa90b63-f287-4467-aa1d-6d59a25313e1`. Other currently
eligible items, in server order, are CAD-95, CAD-130, CAD-88, CAD-91 and CAD-134.
Re-read `ready_work` immediately before selection; do not treat this snapshot as
a lease or bypass a later blocker/conflict.

CAD-96 waits for CAD-95. CAD-93 waits for CAD-88 and CAD-130. CAD-132 is the
integration gate after the remaining generic work, including CAD-134. The intended
sequence is import → MCP and attributable review/history → exterior/section views
→ dimensions/annotations → 3D and drawing exports → staged reference repair →
generic integration. Server readiness/order remains authoritative. Room/MakerSpace
and other specialized workspaces remain held until the generic gate; do not
promote Defined items merely to keep working.

## Resume procedure

1. Read repository `AGENTS.md`, the current PLAINER item and the `plainer-work`
   skill. Preserve `mockup/`, unrelated source, caches and worktrees.
2. Check branch, status and the fresh queue. Claim only one top-level item and
   read it again after claiming. Project ID is
   `3baddf15-4601-4a46-b861-5d8d198229e9`.
3. The prior queue identity was agent `codex-cad-queue`, session
   `cad-queue-20261028`. All its claims are released. Maintain a stable execution
   identity during the resumed run and renew owned leases.
4. Commit coherent steps with Conventional Commits. Update Working Logs with
   actual tests, commits and limitations; submit complete work to Review.
   **Do not push, create remotes, publish or install.**

The host's older broad sketch → solids → exports → Room goal still reports
`blocked`. That is stale host metadata, not an active PLAINER blocker. Earlier
attempts could not resume it programmatically or create a replacement while it
exists. Do not mark the broad goal complete or repeatedly retry goal creation.

## Verified baseline

On production `29a3afa`, formatting, strict locked workspace/all-target Clippy,
locked workspace tests and the native build passed. Tests: **1,307 passed,
zero failed, two existing ignored**. Release contracts: **25/25 passed**.

```sh
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets --locked --offline -- -D warnings
CARGO_INCREMENTAL=0 cargo test --workspace --locked --offline
npm test
CARGO_INCREMENTAL=0 ./build
```

Logs are `/private/tmp/multicad-cad70-handoff-{clippy,tests,build}.log` and
`/private/tmp/multicad-cad70-final-npm.log`. Native Cargo/build commands need
approved access to the shared OCCT cache; a sandbox cache-lock denial is not a
product defect. The known upstream OCCT `sprintf` deprecation remains unchanged.
Use the pinned Rust 1.99.1 toolchain and lockfile. `./build` takes no extra
`build` subcommand. Windows/Linux runtime and low-power performance remain unverified.

Final bundle:
`artifacts/development/build-C0UTpc/MultiCAD.app`, executable SHA-256
`7d1a896c5e73ce80b9f253701593b948b6aac164173446c145fde4f34497e29e`.
It is built but not launched. The actual macOS workflow used
`artifacts/development/build-GlEU4a/MultiCAD.app`, SHA-256
`4090627b7f83eda665cef65b35a9dc0529d32a7520baf82953faf549903a978e`.
Its review instance and older instances were closed through native controls.

Launching a newly built executable triggered automatic approval review despite
the user's broad permission to open apps. The user specifically approved the
GlEU4a and earlier 5ZEsOm builds. A different executable may need its own concrete
launch confirmation. Use CUA for native interactions; do not bypass a rejected
launch or use shell/AppleScript to control applications.

## Symbols and reference architecture

[ADR-0024](../architecture/0024-pinned-symbol-variants.md) explains symbols.
One immutable canonical definition embeds a bounded source template. Each
placement has a fixed private identity map, normal canonical source entities and
an ordinary outer Instance. The definition, overrides and generated graph update
through one Session transaction. Resources are not mutated outside Undo.

Public core APIs live in `crates/multicad-core/src/symbols/`. Definition and
instance collections return `Result<Vec<_>, DocumentError>`. Local sketch IDs in
a map are not Document entity IDs. Generated source is read-only; edit exposed
inputs or explicitly update the pin. Private parameters stay out of Parameters.

Generic Move/Copy now creates an independent symbol variant and rebinds recognized
private pose sources. Do not revert to sharing the original private wrapper.
Parameter normalization now computes all bound updates atomically; incremental
detachment would violate ownership and break derived parameter chains.

Full native evaluation validates unplaced templates too, with count/entity
budgets and driving-reference checks. Source-only views never authorize Apply or
manufacturing. The [occurrence reference contract](../architecture/sketch-external.md)
owns exact query, receipt, source/consumer path and single-reference repair rules.
CAD-134 adds an explicit staged repair batch; do not weaken the complete-model
gate to work around multiple broken references.

Current symbol remappers cover constrained sketches, primitives, extrusions,
ordinary assemblies and supported pose/joint records, basic materials and inline
hatches. Unknown providers and unsupported assigned library metadata reject
explicitly. Nested embedded symbols and generic subassembly alignment remain
outside the supported boundary. No connected MCP server is implemented yet;
CAD-95/96 own that next generic capability.

Small UX follow-ups: the update report retains its original conflict summary
after obsolete overrides are unchecked, although Apply validates the resolved
controls. Some long scrolled Inspector controls required coordinate interaction
because their native accessibility representation lagged. Both are recorded
limitations, not evidence that invalid geometry committed.

## Editable review artifacts

Use `artifacts/symbol-review-20261029-0108/`:

- `part-r1.multicad`, `part-r2.multicad`, `assembly-r1.multicad`,
  `assembly-r2.multicad` and the generator manifest.
- `symbols-place-copy-native-gui.multicad`: four independently parameterized
  instances after physical placement/copy/edit and history checks.
- `symbols-reviewed-upgrade-native-gui.multicad`: explicit revision update after
  removed-override resolution, Apply, Undo and Redo.
- `panel-library-native-gui.multicad`: OS-picker library export, independently
  reopened with production native template validation.
- Corresponding `-report.json` files: real native source regeneration, all
  references resolved, preserved four instance IDs and linked sketches/leaves.

Before upgrade, the original C has width 180 mm and volume 115,200 mm³; its copy
has width 160 mm and volume 102,400 mm³ at a 250 mm Y offset. The revised source
changes both through their retained values and identities. Native two-solid
assembly, face-reference, private-pose, failure and offline tests extend this
physical evidence. See the verification document for exact measurements.

All three helpers are completed; none owns a PLAINER card or runs a build.
Their isolated checkouts are preserved under
`/Users/keller/.codex/worktrees/cad67-assemblies/multicad`,
`cad66-measurements/multicad` and `cad92-planar-dxf/multicad`. Integrated changes
are on main; do not blindly cherry-pick helper histories again. An optional,
unintegrated CAD-128 test patch is preserved at
`/private/tmp/cad128-shared-profile-optional.patch`; it is not required to resume.
