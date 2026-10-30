# Schedules and the BOM

Schedules turn model records into tables. The BOM, the panel cut list, the
machining list, the member cut list, the weld schedule and the bend schedule
all become one `Schedule` type. CSV files and drawing tables are both built
from it, so they always agree.

| Code | Contents |
| --- | --- |
| `crates/multicad-core/src/schedules.rs` | `ScheduleSpec`: which schedule, with the BOM inclusion options |
| `crates/multicad-model/src/schedules.rs` | The BOM, the `Schedule` type, the builders and `schedules::build` |
| `crates/multicad-model/src/export/schedule.rs` | Direct schedule CSV jobs |
| `crates/multicad-mcp/src/tools/schedules.rs` | The MCP `schedule` tool |
| `crates/multicad-ui/src/drawings/schedules.rs` | Drawings → Annotate → Schedule table |
| `crates/multicad-ui/src/exchange/schedule_csv.rs` | Export → Common → Schedule CSV |

## The BOM

`Model::bom` lists the evaluated model:

- **Rows.** One row per Part. Assemblies and groups are never rows.
- **Quantity.** The number of the Part's physical leaf occurrences. A Part,
  its placed copies and every Instance of it in assemblies count once each,
  and nesting cannot count a Part twice.
- **Columns.** Each row holds:
  - the Part's identity and name;
  - its quantity and the occurrence paths behind it;
  - its effective Material and stock;
  - its supply class;
  - its stack, when it is a layer of one.

Rows are sorted by supply class, then name, then identity. The same model
always gives the same BOM.

### Supply classes

| Class | Parts |
| --- | --- |
| Aid | FDM aid Parts |
| Purchased | Parts tagged `purchased` |
| Imported | STEP bodies, KiCad boards and components, unless tagged `manufactured` |
| Manufactured | Everything else |

### Inclusion rules

- **Aids.** Aid Parts are left out unless `include_aids` is set.
- **Purchased and imported Parts.** Included unless `include_purchased` is
  cleared.
- **Scope.** `roots` limits the BOM to selected top-level Parts or Instances.
- **Stack layers.** Always listed individually, each naming its stack.

Every Part that was left out is listed in `excluded` with the reason.

## Schedules

A `Schedule` holds:

- a kind and a title;
- the document revision and the source content hash it was made from;
- notes on units, conventions and inclusion rules;
- column names and text rows.

| Builder | Rows |
| --- | --- |
| `bom_schedule` | Item, Part, quantity, Material, stock, supply, stack |
| `cut_list_schedule` | Each panel: role, stock, thickness, finished and core sizes along and across the grain, grain rule, bands, sheet fit, stack and issues ([cabinetry](cabinetry.md)) |
| `machining_schedule` | Each groove, rabbet or pocket, and each hole size per operation and panel with its count |
| `member_cut_schedule` | Each group of identical member cuts: quantity, section, overall length, both end treatments with the long-point face of each angled cut, and whether two long points lie on the same, opposite or adjacent faces ([weldments](weldments.md)) |
| `bend_schedule` | Each bend of each sheet part: id, proposed order, direction, angle, inside radius, thickness, K-factor, allowance and length ([sheet metal](sheet-metal.md)) |
| `weld_schedule` | Each weld: sequence step and mark, joint kind, members, symbol, size, process and notes |

Lengths are millimeters, rounded to 0.001 mm.

A `ScheduleSpec` names one of the six kinds. For the BOM it also carries
`include_aids` and `include_purchased`; other kinds ignore them.
`schedules::build` turns a spec into a `Schedule` of the whole evaluated
model. MCP, drawing tables, export recipes and direct CSV jobs all call it.

### CSV

`Schedule::csv` writes CSV to RFC 4180. The file starts with `#` comment
lines that give:

- the schedule kind;
- the revision;
- the source hash;
- each note.

A column header line follows, then one line per row. A cell containing a
comma, a quote or a line break is quoted. So is a cell starting with `=`,
`+`, `-` or `@`, so a spreadsheet does not run it as a formula.

`schedules::read_csv` parses this format back. Publication uses it to check
a written file.

### Drawing tables

`Schedule::table` makes a drawing table annotation with the same cells. Its
title names the revision, and its column widths fit the text.
`Schedule::placed_table` does the same and also records the `ScheduleSpec` in
the table's optional `schedule` field. Tables without it, such as hand-made
tables or the weld sequence table, are never checked.

A placed table is page content. It keeps a copy of the cells and does not
update itself. `Schedule::check_table` compares a placed table with a fresh
schedule and reports it as stale when its cells differ. The source hash is
not compared, so edits that leave the rows unchanged, such as placing the
table, do not make it stale.

**Drawings → Annotate → Schedule table.**

1. The command opens a chooser in the Drawing Inspector: the kind, plus the
   BOM options.
2. **Place on page** adds the linked table at the page margin. The whole page
   is committed as one undoable edit. If the current schedule is not built
   yet, the placement waits for it.
3. Each linked table on the page is checked against a fresh schedule of the
   current revision. The inspector shows the selected table as current,
   checking, stale or failed. It also warns when any table on the page is
   stale.
4. **Refresh table** replaces the cells, title and column widths, keeping the
   table's identity, position and row height. It is one undoable edit.

Placing and refreshing need the page to have no unapplied edits. Linked
cells are read-only in the inspector: change the model, then refresh.

The native drawing backend builds the requested schedules on its own model
worker, created on first use. Schedule checks and view previews never
supersede each other.

**Exports.** Placed tables print in [PDF and drawing DXF](drawing-exports.md)
like any table. A PDF or drawing-DXF recipe refuses a page with a stale
linked table. The reason names the page and asks for a refresh, so old rows
are never printed as current.

## Access

- **Drawings.** Schedule table places and refreshes linked tables (above).
- **Export sets.** A Schedule CSV recipe writes one schedule per run; see
  [export sets](export-sets.md#schedule-csv).
- **Export → Common → Schedule CSV.** The command opens a panel with the
  schedule chooser. The output service previews the schedule of the current
  revision off the UI thread, showing its rows and columns.
  1. With a current preview, the command or **Export CSV…** opens the native
     save dialog. It has a CSV filter and suggests `<project>_<kind>.csv`.
  2. The service writes the chosen file only when the CSV it rebuilds hashes
     to the preview. It writes a flushed partial file, reads it back and
     renames it over the target.
  3. Nothing is written, and the reason is shown, when:
     - the document changed while the dialog was open;
     - the preview is not of the current revision;
     - the service finds a different CSV;
     - the path is not an absolute `.csv` file in an existing folder.
  4. An accepted write keeps the window from closing until its outcome is
     read. A command-line path skips the dialog but still needs a current
     preview.
- **MCP.** `schedule` returns any of the six schedules as columns and rows
  with their revision and source hash. With `file_name`, it also writes the
  CSV into the export folder in scope, through a partial file and a rename.
- **API.** `Model::bom`, `schedules::build` and the builders in
  `multicad_model::schedules`; `Exporter::schedule` and
  `ExportWorker::schedule` for CSV jobs.

## Verification

- `crates/multicad-model/tests/schedule_outputs.rs` covers the native
  outputs:
  - recipe CSV files equal `Schedule::csv` and the bundle is reused;
  - placed tables print in PDF and DXF;
  - a new Part blocks both drawing formats until the table is refreshed;
  - a direct write with an outdated hash writes nothing;
  - invalid targets are refused.
- UI unit tests use controlled services. They cover waiting placement, the
  single Undo step, stale status and refresh. For Schedule CSV they cover
  preview-before-write, foreign replies, stale refusals, the CSV save dialog,
  the close guard and an edit while the dialog is open.
- `multicad-app` tests drive the production adapters: a real BOM table is
  placed, goes stale and is refreshed, and a real BOM CSV is written through
  the export worker.

These checks do not include an interactive macOS review of the panels.

## Limits

- **Classification.** Supply classes come from Part tags and import
  features. There is no purchasing catalog, supplier, part number or cost.
- **No live tables.** Placed tables are copies. They are reported stale but
  never regenerated without an explicit Refresh.
- **Whole project.** Schedules from recipes, Schedule CSV and Drawings
  always cover the whole project. The export set's Part scope does not apply,
  and BOM `roots` are not offered in the window.
- **Size.** A drawing table holds at most 256 rows and 32 columns with cells
  of up to 256 bytes. A larger schedule cannot be placed and says so; its CSV
  can still be written.
