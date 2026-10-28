# Native planar DXF checkpoint, 2026-10-28

The production macOS application exported selected native panel faces as real
DXF files, reused an unchanged bundle and persisted an explicit face selection.
A separate developer-only reader imported the actual files and checked their
geometry against saved recipes and receipt hashes. This records agent
verification; human acceptance remains separate.

## Executable and source evidence

The native UI session used `artifacts/development/build-Fd6A5g/MultiCAD.app` from
source `e1ce67f`. Its `Contents/MacOS/multicad` SHA-256 was independently checked:

`2bb6d06193518d80feec245a6383e00ff74abc49214ae3405a173c1d86893c1d`.

Paths for native assets below are relative to
`/Users/keller/repos/_apps/multicad`. The input project was
`artifacts/planar-ui-20261028/panel-review.multicad`: a 120 × 80 × 18 mm panel
with two circular openings and a rotated, translated occurrence.

The canonical input hash recorded in both initial export manifests was
`23b4bcbbe0dc303515c00601cf176fe2f3dcc77f0a5e9b05d9bbec0af3874716`.
Both manifests attributed the operation to `local-user` through channel `ui`.
The source remained at revision 0 during those exports.

## Native selection, export and reopen

Export, Planar panels displayed two Ready files with planar loop information.
Run published both files beneath `artifacts/planar-ui-20261028/native-ui/`.
Repeating Run displayed “Reused 2 files” in the same owned bundle.
Placed panel inches displayed the explicit Inch unit and published one file.

Choose panel face initially displayed Unsupported and disabled Run. Load current
planar faces returned the actual finite faces. The operator selected the face
labelled Face 4 with normal `[0, 0, 1]`.

Saving WorldZ as the horizontal axis produced revision 1 and the diagnostic
“Chosen horizontal world axis is normal to this face; choose another axis”.
Run remained disabled. After reloading the current face catalog, the operator
selected WorldX and saved revision 2. The recipe became Ready and Run published
one actual DXF.

Save As wrote `artifacts/model-review/planar-dxf-native.multicad`. Its canonical
source hash was
`c70ea98f3f6f522b299a9dbb80a3c27a0150d61932ecbe2a3197f18b2811ed88`.
The review app quit before a fresh launch opened that saved file at revision 0.
Choose panel face remained Ready with the same content-derived filename and
persisted loop information.

## Independent file checks

The [reader](../../crates/multicad-model/examples/planar_dxf_file_review.rs)
uses the pinned developer-only Rust `dxf` 0.6.2 dependency. It reads the saved
project and recipe, imports the actual DXF and compares byte counts and SHA-256
with `multicad-export.json`. It reconstructs bounds and signed areas directly
from imported vertices and bulges, independently of the native contour writer.

All four inspected files contained one closed counterclockwise outer contour
and two closed clockwise circular openings. Their outer area was 9,600 mm²;
the opening radii were 5 and 3 mm. Net area was 9,493.185849777949 mm² within
the reader's 0.000001 mm² comparison tolerance.

| Actual output | Declared units | Bounds in model mm | Opening centers in model mm |
| --- | --- | --- | --- |
| Panel definition | Millimeters | `[0, 0]` to `[120, 80]` | `[30, 20]`, `[80, 50]` |
| Placed occurrence | Millimeters | `[120, 100]` to `[200, 220]` | `[180, 130]`, `[150, 180]` |
| Placed occurrence | Inches | Same placed bounds after multiplication by 25.4 | Same placed centers |
| Explicit face after Save As | Millimeters | Same definition bounds | Same definition centers |

Each circle remained two analytic semicircles. No cut segment had zero length
or non-finite coordinates. `CUT` contained the outer rectangle; `CUT_INNER`
contained the openings. Two POINT entities on `CONSTRUCTION` matched their
centers, and one TEXT entity on `ANNOTATION` contained “Cabinet side”.

| File content | SHA-256 matching its native receipt |
| --- | --- |
| Definition and explicitly selected face | `d724944f39de7aa519033140b66a05401f1275791708ca816dece15a75e15494` |
| Placed occurrence, millimeters | `838209b4ba38d7e0b5be5c1907520885d8d34362494bdddaf2f8eff3d27ed6c6` |
| Placed occurrence, inches | `4035f2378e5364295010f655738c7294417df4c4e0d27a9812fc740f9363e2ae` |

Independent reports are retained under
`/Users/keller/.codex/worktrees/cad92-planar-dxf/multicad/artifacts/planar-dxf-review/`
as `native-ui-mm-report.json`, `native-ui-inch-report.json` and
`native-ui-face-report.json`. The main checkout also retains
`artifacts/planar-ui-20261028/native-ui-face-report.json`.

The executed command accepts the source project, owned bundle directory and a
new report path:

```sh
cargo run -p multicad-model --example planar_dxf_file_review --locked --offline -- \
  SOURCE.multicad BUNDLE_DIRECTORY NEW_REPORT.json
```

Disposable copies confirmed that a same-length file modification fails the
receipt hash check. A foreign saved source failed its source hash check.
An existing report caused File exists and retained its original hash; rejected
audits created no report. `native-ui-audit-guards.json` records those checks.

## Automated evidence and limits

The integrated application checks in
`/private/tmp/multicad-coalesced-drag-{fmt,clippy,tests}.log` passed formatting,
strict workspace Clippy and the locked offline workspace suite: 948 passed,
zero failed, two existing ignored. The isolated snapshot containing the audit
helper passed the same required checks with 877 passed, zero failed and two
ignored. Its logs are `/private/tmp/multicad-planar-file-review-{clippy,tests}.log`.

The [planar export contract](../architecture/planar-dxf.md) defines supported
curves, coordinates, diagnostics and the developed-contour provider boundary.
This checkpoint establishes neither sheet unfolding nor CNC toolpaths. Internal
face contours do not infer machining depth. Windows/Linux interaction,
low-power performance and manufacturing acceptance remain unverified.
