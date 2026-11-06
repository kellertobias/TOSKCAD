# Sourced screw heads and recess profiles — 2026-11-06

CAD-236 adds 52 metric screw presets, real native pan/button/countersunk
heads and hexalobular/Phillips H recesses to the canonical mechanical-part
recipe, Library, native UI and MCP. Implementation commit:
`490ae9d0a5e2d9e0aed8a680d1f36068fb523e04`, on the existing `main` branch.
Engineering verification passes as recorded below. Human review and acceptance
remain separate.

The bundled Library now contains 128 projects. All 76 previous project bytes,
manifest identities and hashes remain unchanged, including the nineteen
original machine screws. No push, installation or publication was performed.

## Source coverage and retained meaning

The canonical screw curation retains legacy revision 1 and adds profile
revision 1. Its 96 family/nominal cells cover all twelve requested diameters:
2/2.5/3/3.5/4/4.5/5/6/7/8/9/10 mm. Fifty-two cells have selected sourced rows;
44 explicitly describe unavailable choices within this curation. They make no
claim about global commercial availability.

| Source family | Head / drive | Selected diameters, mm | Selected lengths, mm | Convention |
| --- | --- | --- | --- | --- |
| ISO 14583:2011 | Pan / hexalobular | 2, 2.5, 3, 3.5, 4, 5, 6, 8, 10 | 12 | Underhead |
| ISO 14581:2013 | Countersunk / hexalobular | Same nine | 12 | Overall |
| ISO 7045:2011 | Pan / Phillips H | Same nine | 12 | Underhead |
| ISO 7046-1:2011 | Countersunk / Phillips H | Same nine | 12 | Overall |
| ISO 7380-1:2011 | Button / internal hex | 3, 4, 5, 6, 8, 10 | 12; M10 uses 16 | Underhead |
| ISO 10642:2004 | Countersunk / internal hex | 3, 4, 5, 6, 8, 10 | 12 | Overall |
| FILA ISO 14579 sheet, undated | Socket cap / hexalobular | 6, 8, 10 | Respectively 30, 40, 40 | Underhead |
| Accu SHB-M3-10-A2-R360 | Button / hexalobular | 3 | 10 | Underhead |

Historical editions remain named. ISO 10642:2004 does not acquire the BOLTS
DIN 7991-like M2/M2.5 rows. Accu's identified product is separate from the
internal-hex ISO 7380 family. Product size-45 A-reference 7.95 and hexalobular
basic A 7.93 remain separate. Original head limits, theoretical/actual
diameters, functional gauges and chosen engineering contours are not merged.

Eleven primary references retain URLs, editions, research-copy hashes and page
references. Unmodified BOLTS starting data and LGPL notices remain bundled.
Source PDFs, diagrams, artwork, supplier CAD and FreeCAD code are not
redistributed. The new source notice identifies independent scalar adaptations
and engineering targets. These are nominal-basic threads and dimensional
references, without fit-class, current-standard, normative gauge, physical
driver, strength, material, coating, stock or print-readiness certification.

The exact-one length contract preserves legacy `underhead_length` serialization
and meaning. New definitions retain one tagged Underhead/Overall Dimension;
Overall subtracts the physical head height to derive the neck-to-tip span.
The threaded extent remains a bound expression, subtracting the retained plain
transition and head where required. Each new pin has one Count representation
input. Immutable source references remain literal and unbound. Reader
capabilities cover new retained lengths and profiles in templates, private
instances, references and suppressed/nested source.

## Native geometry and failure cases

Pan/button/countersunk targets retain physical front lands, real tangent neck
blends and declared crown/rim/cone/edge geometry. The FILA legacy socket-cap
envelope has no invented neck-radius input. Hexalobular cuts use twelve tangent
circular arcs. Phillips H0–H4 use the actual inclined C–C section, H0 rounded
inner form or H1–H4 shallow V, and separate mouth roundover. Gauge penetration
and deepest removed point retain distinct meanings.

The native builder uses existing audited geometry APIs; no C++/FFI or trusted
thread-factory source changed. The recess is cut after thread fusion so a
legitimate countersunk cavity into the neck remains open. Completed-part cache
keys include every resolved physical input. An actual warm shared-factory
source test preserves BRep bytes, volume, bounds, topology and provenance
through new head/recess Boolean and STEP operations.

The first broader native test found a real finite cutter-cone closure inside a
pan head. The missing 0.2917096 mm functional slice explained the independently
calculated volume difference. Extending the nonfunctional closure above the
prism fixed it; native top-face void probes were added. Original failed output
and the corrected six-case result are retained. No oracle or tolerance changed.

A separate counterexample proved that a positive 0.005 mm annulus could pass
under a configured native minimum feature of 0.01 mm. Native preflight now
checks the physical annulus, blind floor and complete piecewise body/recess
envelopes before cache/native work. The thin-annulus/floor failures and a valid
countersunk Phillips cavity into the neck pass their respective negative and
positive tests. This applies the application's policy without inventing an ISO
wall limit.

## Packaged runtime and actual macOS workflow

The release package reports exact source commit `490ae9d`; executable SHA-256
is `c94a304d3d8423beb4f2076cede8957c19e91c12f7d84dd984f093c09adc1f70`.
Its self-test passes. A sandbox cache-lock denial and a successful but unstamped
first build are retained separately; neither was substituted for the correctly
stamped runtime.

Public packaged MCP creates all eight families with bound lengths, rejects
wrong-unit/incompatible/stale edits atomically, edits a head parameter through
Undo/Redo, saves, exports eight STEP files and reopens in a separate read-only
process. Full source/resolved values, physical counts, bounds, volumes and file
hashes are checked. The first harness expected the wrong error code for an
incompatible pair; the application correctly returned `invalid_document`.

The second harness used an additional relative-volume-only STEP diagnostic.
Countersunk H1 exceeded that partial budget. Independent native/STEP area
measurement confirms all eight exports pass the unchanged complete policy.
The independent verification budget is
`ε × (A_native + A_STEP) + r × (|V_native| + |V_STEP|)`, using the unchanged
`ε = 1e-7 mm` and `r = 1e-7`. For H1, delta is `4.15428225e-5 mm³`, below
`5.92837194e-5 mm³`: both measured areas are approximately 213.8719 mm² and
supply the omitted `4.27743799e-5 mm³` area term. H1 bounds differ by at most
`1.90e-13 mm`; the maximum across all eight exports is `1.47e-12 mm`. Each has
one native and one STEP solid. Thirty-six H1 material/void probes pass. Its
retained attempts 02 and 03 have identical measured STEP geometry. This
comparison is performed by the independent audit, rather than an additional
delta guard in `export_step`. No application or kernel tolerance was changed.

Actual Computer Use operated the packaged macOS application. A custom Pan/H1
preview cancelled at revision 0; Apply/Undo/Redo produced revisions 1/2/3 and
the atomic save captured its native preview at revision 4. Head/drive repair,
scrolling and fixed Preview/Apply/Cancel actions were exercised in a compact
1100 × 700 logical window. Undo/Redo evidence uses action buttons; keyboard
shortcuts are not claimed by this check.

The included ISO 14581 M3 × 12 pin was placed at world X=20 with Overall 14.
Cosmetic representation applied and Print explicitly refused manufacturing.
Resetting its override inherited Modeled while preserving the original pin,
instance, 18 private IDs, 16 inputs, length and pose. Independent saved-source
checks verify Overall 14 → underhead 12.35/thread 11.725/plain 0.625 mm, an
unchanged literal Overall-12 reference and custom dimensional attribution.

The native UI wrote and read back a 3MF with two objects/build items, one placed
occurrence and 20,580 triangles. Raw ZIP/XML checks independently verify closed,
consistently wound meshes, single connected components and bounds/volumes
within the existing chord-0.1 budgets. Negative head Z is retained and reported;
the package does not establish orientation, supports or physical printability.

A fresh GUI process reopened both objects and the same pin/Overall-14/default
Modeled state. Cancelling inspection preserved revision 0 and the saved bytes.
Both owned GUI processes exited normally; Sky then reported no running ToskCAD
windows. Preferences remained byte-exact with their original SHA-256
`49f7596bb838b8f71664a313bf91e9ba8a760d3393bd59179f99a41a0c056bb7`;
no restore write was needed and GUI MCP settings were not changed. Functional
checks ran alongside the full suite and are not isolated latency measurements.

## Verification scope

Formatting and strict locked workspace/all-target Clippy pass. Packaging and
release contracts pass 34/34, including missing-notice, source hash, coverage,
identity and resource-copy controls. Focused core/UI/MCP/native/catalog receipts
retain failures and final results separately. Catalog generation actually built
and rendered all 52 new native projects. Regeneration now accepts byte-identical
existing pins and refuses changed identities, bytes or geometry without an
explicit migration.

The complete locked workspace run passed **2,803 tests, zero failures and four
existing ignores across 327 groups**, exit 0. It started at 14:17:04 UTC and
finished at 15:35:03 UTC. Its command was
`cargo test --workspace --locked --no-fail-fast -- --test-threads=4`, with test
optimization level 3, debug assertions and overflow checks enabled, and debug
information disabled. The pinned Rust toolchain and committed lockfile were
used. Source remained at `490ae9d` with no tracked differences throughout the
run; separate release builds used a different Cargo target directory.

The final run includes four catalog cases, five core profile cases, four native
profile cases, two public MCP profile cases and the complete UI package. The UI
library passed 1,013 tests with two existing ignores. Executed binary hashes and
its actual test listing are retained with the full-workspace summary; the UI
binary SHA-256 is
`2d43638bdc8a1c0909044367fbf5487e012ce2529bc82fd8497063652f47689b`.
Focused runs overlap these results and are not added as unique passes.

After the frozen-code run, two README facts were corrected: neck blends apply
to pan/button/countersunk targets rather than the FILA legacy caps, and new
countersunk identity keys use authored overall length. The final bundle has
exactly one changed Library resource, `README.md`; all other resource/project
bytes match the fully exercised bundle. Executable content outside the
code-signature command and signature blob is byte-identical. Its signed
executable SHA-256 is
`e4353a757f72fcf725600c46bf19410d334c85c567048aaf1c555ba3797ce69f`.
The final bundle separately passes its self-test and a fresh native GUI reopen
showing both modeled screws, then exits normally. Saved source and preferences
remain byte-exact and no ToskCAD windows remain open. The report and README
corrections are documentation changes after the tested implementation commit;
no second full Rust run on that documentation commit is claimed.

No current Windows/Linux runtime or CI result is inferred from macOS evidence.
No required implementation or verification remains for this bounded CAD-236
scope; broader catalog/BOM/connection work remains in its own PLAINER items.

## Evidence index

- [Canonical curation](../../library/machine-screws-curation.json),
  [source/adaptation notices](../../library/sources/bolts/SCREW-PROFILE-NOTICES.md),
  [retained-profile design](../architecture/0038-retained-screw-head-and-recess-profiles.md).
- [Generated-project integrity](../../artifacts/screw-heads-20261106/generated-01-integrity.json),
  [root independent packaging audit](../../artifacts/screw-heads-20261106/checks/generated-independent-audit.json),
  [independent source review](../../artifacts/screw-heads-20261106/independent-source-catalog-handoff.json).
- [Initial profile/legacy regression](../../artifacts/screw-heads-20261106/checks/native-profiles-regression.log),
  [minimum-feature counterexample](../../artifacts/screw-heads-20261106/checks/native-min-feature-counterfactual.log),
  [corrected policy case](../../artifacts/screw-heads-20261106/checks/native-min-feature-fixed.log).
- [Packaged public MCP lifecycle](../../artifacts/screw-heads-20261106/public-runtime-03/receipt.json),
  [independent full STEP budget](../../artifacts/screw-heads-20261106/public-runtime-03/independent-full-step-budget.json),
  [H1 material probes](../../artifacts/screw-heads-20261106/public-runtime-03/independent-step-material-probes.json),
  [retained-attempt diagnosis](../../artifacts/screw-heads-20261106/independent-step-diagnosis-summary.json).
- [Actual UI lifecycle and cleanup](../../artifacts/screw-heads-20261106/native-ui-lifecycle.json),
  [saved canonical UI audit](../../artifacts/screw-heads-20261106/qa-ui/independent-canonical-ui-audit.json),
  [raw 3MF audit](../../artifacts/screw-heads-20261106/qa-ui/independent-actual-ui-print-audit.json).
- [Formatting](../../artifacts/screw-heads-20261106/checks/implementation-format-final.log),
  [strict workspace Clippy](../../artifacts/screw-heads-20261106/checks/implementation-workspace-clippy-first.log),
  [packaging contracts](../../artifacts/screw-heads-20261106/checks/package-contracts-first.log),
  [terminal full-workspace source scope](../../artifacts/screw-heads-20261106/checks/full-workspace-scope.json),
  [full-workspace result and binary hashes](../../artifacts/screw-heads-20261106/checks/full-workspace-summary.json),
  [executed UI test listing](../../artifacts/screw-heads-20261106/checks/full-workspace-ui-test-list.txt).
- [Initial fully exercised runtime](../../artifacts/screw-heads-20261106/runtime.json),
  [final documentation/resource closure](../../artifacts/screw-heads-20261106/final-resource-closure.json),
  [final packaged self-test](../../artifacts/screw-heads-20261106/checks/packaged-self-test-final.log),
  [final native reopen](../../artifacts/screw-heads-20261106/native-final-reopen-rendered.jpeg).
