# Native STEP export checkpoint, 2026-10-28

The production macOS application exported three actual STEP files from a saved
tag-filtered set. Independent native reads confirmed valid solids, volumes and
world-space bounds. This verifies the STEP provider and export-set workflow;
the complete exchange format suite and human acceptance remain separate.

## Executable and selection evidence

The review used `artifacts/development/build-1VdX8T/MultiCAD.app`, from `edf925b`.
Its `Contents/MacOS/multicad` SHA-256 was
`89d25c55383f3a8393190f9025ad2b45fe31759eff2fd2eeac260df627d3d6c0`.
The initial input was `/private/tmp/cad90-export-review.multicad`.

In Export, the saved CNC wood set selected tag `cnc` across the whole project.
Preview resolved two source Parts and one separately placed Shelf occurrence.
Run exported them at source revision 0; the receipt identified actor
`local-user` with channel `ui`.

| Exported subject | Volume, mm³ | Minimum world bounds, mm | Maximum world bounds, mm |
| --- | ---: | --- | --- |
| Cabinet side | 172,800 | (0, 0, 0) | (120, 80, 18) |
| Shelf definition | 144,000 | (150, 0, 0) | (250, 80, 18) |
| Placed Shelf | 144,000 | (150, 120, 0) | (250, 200, 18) |

The placed output retained a distinct occurrence path and world solid identity.
The saved assembly-scoped set preview resolved only that placed Shelf.
A second CNC wood Run reported Reused for the same three-file owned bundle,
without changing canonical source.

## Written files and independent native reads

The owned output directory contained three `.step` files and
`multicad-export.json`:

```text
artifacts/export-review/native-step-20261028/
  CNC_wood_687f65f5-13fe-42f0-923b-ae0ec2579bb6_ebfbc73a2894_e7146ef0/
```

The receipt records source and plan hashes, canonical subject identities,
occurrence paths, output names and per-file content hashes. Independent SHA-256
and byte-count comparisons matched every recorded file.

The temporary Rust helper `/private/tmp/multicad-step-file-review.rs` reopened
each file with `Solid::read_step` from the built native kernel. That API validates
the B-rep and rejects non-volumetric or non-positive results. Each file contained
one solid; independently measured values matched the table within 0.000001 mm
for bounds and 0.000001 mm³ for volume. The detached results are
`/private/tmp/multicad-step-file-review.jsonl`.

## Unsupported formats and automated evidence

Saving a recipe change to 3MF committed revision 1. Preview explicitly reported
an unavailable 3MF provider and disabled Run. During editing, the previous preview
was labeled display-only. Undo at revision 2 restored the STEP recipe and its
Ready status with Run enabled.

Save As wrote `artifacts/model-review/export-sets-native.multicad` at revision 2.
Application inventory confirmed all review instances stopped after saving.
A fresh interactive reopen remained pending at this checkpoint.

The integrated checkpoint passed `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --locked --offline -- -D warnings`, and
`cargo test --workspace --locked --offline`: 865 passed, zero failed, two ignored.
Logs are `/private/tmp/multicad-native-workflow-integrated-{fmt,clippy,tests}.log`.

These checks establish real STEP output for the recorded subjects. They do not
establish the other planned 3D formats, PDF/DXF drawing writers, Windows/Linux
interaction, low-power performance or completion of all model exports.
