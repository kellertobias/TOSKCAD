# Schedules and the BOM

Schedules turn model records into tables. The BOM, the panel cut list, the
machining list, the member cut list and the weld schedule all become one
`Schedule` type. CSV files and drawing tables are both built from it, so they
always agree.

| Code | Contents |
| --- | --- |
| `crates/multicad-model/src/schedules.rs` | The BOM, the `Schedule` type and the schedule builders |
| `crates/multicad-mcp/src/tools/schedules.rs` | The MCP `schedule` tool |

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
| `member_cut_schedule` | Each group of identical member cuts: quantity, section, overall length and both end treatments ([weldments](weldments.md)) |
| `weld_schedule` | Each weld: sequence step and mark, joint kind, members, symbol, size, process and notes |

Lengths are millimeters, rounded to 0.001 mm.

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

### Drawing tables

`Schedule::table` makes a drawing table annotation with the same cells. Its
title names the revision, and its column widths fit the text.

A placed table is page content. It does not update itself.
`Schedule::check_table` compares a placed table with a fresh schedule and
reports it as stale when its cells differ.

## Access

- **MCP.** `schedule` returns any of the five schedules as columns and rows
  with their revision and source hash. With `file_name`, it also writes the
  CSV into the export folder in scope, through a partial file and a rename.
- **API.** `Model::bom` and the builders in `multicad_model::schedules`.

## Limits

- **Classification.** Supply classes come from Part tags and import
  features. There is no purchasing catalog, supplier, part number or cost.
- **Sheet metal.** There is no bend schedule yet; it waits for the
  sheet-metal workbench.
- **No live tables.** Tables are not regenerated automatically, and no
  window command places a schedule table yet.
- **No export sets.** Export sets have no CSV recipe yet; CSV files are
  written through MCP.
