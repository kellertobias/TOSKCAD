# Native FDM aid verification, 2026-11-05

Actual macOS interaction verified distinct Tab, Bridge and Support choices,
viewer picks, native contact checks, creation and saved edits. All three passed
Preview/Cancel, Apply, Undo/Redo and cold reopen. Export previews retained the
aid inclusion policy. Human acceptance remains separate.

This supplies the window-interaction evidence missing from the
[historical report](fdm-aid-clarity-2026-11-02.md).
The form is implemented in
[`fdm_aid.rs`](../../crates/multicad-ui/src/modeling/fdm_aid.rs), with viewer
placement in [`placement.rs`](../../crates/multicad-ui/src/modeling/fdm_aid/placement.rs).
The owned evidence root is `artifacts/fdm-aid-review-20261105/`.

## Choices, picking and contact checks

Model → Other showed Breakaway tab, Thin bridge and Removable support with
distinct icons. Each opened its corresponding form and guidance.
Switching a new form from Tab to Bridge to Support changed the relevant fields
and pick requirements without committing source. Preview stayed disabled until
a viewer pick finished or Use coordinates was chosen explicitly.

Tab described a horizontal overlapping pad that can be broken away.
A side-face pick set its base contact and outward direction.
The native check reported exact contact at (0, 10, 0) mm and 180°.
Bridge described a thin bar to cut away between two surface anchors.
Its first pick requested an end pick; clicking the other finished body selected
Second finished block automatically. The picked Bridge passed native Preview.

Support described a block whose height follows the finished part's underside,
with a deliberate top gap. A viewer footprint pick passed the native check,
which reported 0.200 mm separation. These are geometry and removal-intent checks;
physical printing and removal were not tested.

An existing Tab reopened with its saved type locked while dimensional editing
remained available. Moving its contact X to 1000 mm produced a native preview
with an explicit refusal: the Tab was 952.000 mm from the finished part.
Apply stayed disabled even though preview geometry was available.
Cancel preserved revision 0 and the saved file.

## Actual creation, history and independent geometry

The [fixture preparation receipt](../../artifacts/fdm-aid-review-20261105/independent-finished-only-fixture-preparation.json)
records the three starting projects. A separate public typed MCP transaction
removed each old aid from an owned copy and saved a finished-only project.
No container or source JSON was rewritten manually.
The remaining canonical entities and native finished geometry stayed unchanged.
Original historical fixtures were preserved.

Each new aid was then authored in the native window from a finished-only copy.
Viewer picking was checked before the final exact coordinate recipes.
For each type, Preview/Cancel preserved revision 0.
Repeating the form committed one aid at revision 1.
Visible toolbar Undo removed it at revision 2; Redo restored revision 3.
Save retained revision 4. A saved dimensional edit then passed Preview/Apply
at revision 5 and Save at revision 6.

| Aid | Exact creation recipe | Saved edit | Final native and STEP volume, mm³ |
| --- | --- | --- | ---: |
| Tab | Contact (0,10,0), direction 180°, reach 8, overlap 2, width 6, thickness 0.4 mm | Reach 10 mm | 28.8 |
| Bridge | Start (39,10,0.2), end (61,5,0.2), two finished Parts, width 2, thickness 0.4 mm | Width 2.5 mm | 22.5610283454 |
| Support | Corner (5,5), footprint 30 × 10, base 0, gap 0.2 mm; underside Z=15 mm | Gap 0.3 mm | 4410 |

Independent current-runtime audits check the actual GUI-saved sources and STEP
write/readbacks: [Tab](../../artifacts/fdm-aid-review-20261105/independent-native-tab-created-readback.json),
[Bridge](../../artifacts/fdm-aid-review-20261105/independent-native-bridge-created-readback.json)
and [Support](../../artifacts/fdm-aid-review-20261105/independent-native-support-created-readback.json).
Each aid remains a separate Part with its own feature identity.
The original finished body stays at 8000 mm³ in each project.
Bridge's second finished body stays at 1000 mm³.

The final Tab bounds are [-10,7,0] to [2,13,0.4] mm.
Its native contact overlap is 4.8 mm³.
Bridge retains both target identities and valid overlapping contacts.
Support has bounds [5,5,0] to [35,15,14.7] mm, zero target overlap and a
measured 0.3 mm gap. All three final native contact reports have no issues.
Audit reads and exports leave their input source hashes unchanged.

## Export, recall and discoverability

Export → Print retained the separate inclusion policy for removable aids.
On the original Tab fixture, All plus Include removable FDM aids previewed two
objects totaling 8024.0 mm³. Turning the option off previewed only the finished
body, totaling 8000.0 mm³, with an explicit explanation for the omitted Tab.
Canceling the Write 3MF destination dialog wrote no package.
Choosing None produced a missing-parts message and disabled writing.
These checks left revision 0 unchanged.

The edited Bridge's print preview contained three objects totaling 9022.6 mm³,
including both finished bodies and the associated aid.
The edited Support's preview contained two objects totaling 12410.0 mm³.
These checks verify the native package preview and policy, without claiming a
newly written manufacturing file or printer readiness.

Each saved created project was reopened after a fresh application launch.
Edit latest feature retained the aid type, target choices and edited dimensions.
Support retained its footprint and gap; Bridge retained its second Part and
width; Tab retained its reach. Each editor was canceled at revision 0.

Search found tab, bridge and support under Model → Other.
Selecting Support from Common opened the correct form in Other.
Settings displayed the current Other label. After temporarily hiding Other,
search still opened Bridge and exposed its configuration.
The original visible preference was restored before closing Settings.
Automated legacy FDM/FDM aids recall and shortcut guards remain separate
coverage; no native custom-shortcut binding round trip is claimed.

## Layout, checks and limits

The Support editor and native preview were checked at the current minimum
1100 × 700 logical size, with scrolled fields and reachable Preview/Apply/Cancel.
Normal captures are 2720 × 1720 pixels; compact captures are 2200 × 1400.
The final UI consistency item owns the smaller 1024 × 600 target.
The [capture manifest](../../artifacts/fdm-aid-review-20261105/capture-manifest.json)
indexes 51 JPEG/accessibility pairs, including contact failures and cold recall.

The [independent visual review](../../artifacts/fdm-aid-review-20261105/independent-fdm-aid-visual-review.json)
inspected 19 captures and found the distinct controls, contact refusals and
compact footer readable. A remaining presentation detail is assigned to the
final UI pass: after switching a new Tab form to Bridge or Support, the bottom
status retains the launch label while the form, guidance and ribbon highlight
show the current type correctly.

The runtime is the optimized bundle built from `3a9da7a`, executable SHA-256
`e2bc46957fd6365871d9137e9791bfa1899171e23f50de3ea057f795d1f073f9`, at
`artifacts/wood-navigation-review-20261105/optimized-final/MultiCAD.app`.
All independent native receipts identify this same executable.
Diagnostics reports the source commit as not stamped; this is local evidence.

No Rust changes were required. The unchanged Rust tree reuses the completed
[check receipts](../../artifacts/wood-navigation-review-20261105/checks/summary.json):
formatting, strict locked all-target workspace Clippy, release build, `./build`,
version and diagnostics passed. `cargo test --workspace --locked` exited zero:
2591 passed, zero failed and four intentional ignores across 295 result groups.
The run used optimization level 3 with debug assertions and overflow checks.
Its UI group passed 961 tests with two GPU ignores in 63.63 seconds.

The final [running-window inventory](../../artifacts/fdm-aid-review-20261105/closed-review-windows.json)
is empty. No Windows/Linux interaction, physical print/removal, installation,
push or publication is claimed. Work remains local on main, with unrelated
`.claude/` content preserved.
