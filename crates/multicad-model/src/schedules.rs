//! Schedules: the BOM and domain cut lists as one record type that both CSV
//! files and drawing tables are built from. Every schedule carries the
//! document revision and source hash it was produced from, so a placed
//! table can be checked against a fresh one.
use crate::{CutRecord, MachiningRecord, Model, ModelError, error};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::AnnotationKind,
    materials,
    model::{self, OccurrencePath, PanelRole},
    project_file::ContentHash,
};
use std::collections::BTreeMap;

/// How a Part is supplied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Supply {
    Manufactured,
    /// Tagged `purchased`.
    Purchased,
    /// A STEP or KiCad import, unless tagged `manufactured`.
    Imported,
    /// An FDM aid Part, removed after printing.
    Aid,
}
impl Supply {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manufactured => "manufactured",
            Self::Purchased => "purchased",
            Self::Imported => "imported",
            Self::Aid => "aid",
        }
    }
}
/// What a BOM includes. Assemblies and groups are never rows: every row is
/// a Part, counted once per physical leaf occurrence.
#[derive(Clone, Debug)]
pub struct BomOptions {
    pub include_aids: bool,
    pub include_purchased: bool,
    /// Limit the BOM to these top-level Parts or Instances.
    pub roots: Option<Vec<EntityId>>,
}
impl Default for BomOptions {
    fn default() -> Self {
        Self {
            include_aids: false,
            include_purchased: true,
            roots: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct BomRow {
    pub part: EntityId,
    pub name: String,
    pub quantity: usize,
    pub occurrences: Vec<OccurrencePath>,
    pub material: Option<(EntityId, String)>,
    pub stock: Option<(EntityId, String)>,
    pub supply: Supply,
    /// The stack the Part is a layer of; layers are listed individually.
    pub stack: Option<(EntityId, String)>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Bom {
    pub revision: u64,
    pub source: ContentHash,
    pub rows: Vec<BomRow>,
    /// Parts left out by the options, with the reason.
    pub excluded: Vec<(EntityId, String)>,
}

fn name(document: &Document, id: EntityId) -> String {
    document.get(id).map(|e| e.name.clone()).unwrap_or_default()
}
fn supply(
    document: &Document,
    part: EntityId,
    aids: &std::collections::BTreeSet<EntityId>,
) -> Supply {
    let Some(EntityData::Part { features, tags, .. }) = document.get(part).map(|e| &e.data) else {
        return Supply::Manufactured;
    };
    if aids.contains(&part) {
        return Supply::Aid;
    }
    if tags.contains("purchased") {
        return Supply::Purchased;
    }
    let imported = features.iter().any(|f| {
        matches!(document.get(*f).map(|e| &e.data), Some(EntityData::Feature { definition, .. })
            if [model::imports::STEP_BODY_TYPE, model::imports::BOARD_TYPE, model::imports::COMPONENT_TYPE]
                .contains(&definition.type_id.as_str()))
    });
    if imported && !tags.contains("manufactured") {
        Supply::Imported
    } else {
        Supply::Manufactured
    }
}

impl Model {
    /// The BOM of the evaluated model: one row per Part, quantity from its
    /// physical leaf occurrences, sorted by supply then name.
    pub fn bom(&self, document: &Document, options: &BomOptions) -> Result<Bom, ModelError> {
        let aids = model::aid_parts(document);
        let resolver = materials::Resolver::new(document).map_err(|e| error(None, e))?;
        let mut stacks = BTreeMap::new();
        for s in model::stacks(document) {
            for layer in model::read_stack(document, s.id)
                .map_err(|e| error(Some(s.id), e))?
                .layers
            {
                stacks.insert(layer, (s.id, s.name.clone()));
            }
        }
        let mut found: BTreeMap<EntityId, Vec<OccurrencePath>> = BTreeMap::new();
        for (path, leaf) in &self.snapshot().leaf_occurrences {
            if options
                .roots
                .as_ref()
                .is_some_and(|r| !r.contains(&path.root))
            {
                continue;
            }
            found.entry(leaf.component).or_default().push(path.clone());
        }
        let mut rows = vec![];
        let mut excluded = vec![];
        for (part, occurrences) in found {
            let supply = supply(document, part, &aids);
            if supply == Supply::Aid && !options.include_aids {
                excluded.push((part, "FDM aid".into()));
                continue;
            }
            if matches!(supply, Supply::Purchased | Supply::Imported) && !options.include_purchased
            {
                excluded.push((part, format!("{} Part", supply.label())));
                continue;
            }
            let props = resolver.resolve(part).map_err(|e| error(Some(part), e))?;
            rows.push(BomRow {
                part,
                name: name(document, part),
                quantity: occurrences.len(),
                occurrences,
                material: props.material.map(|m| (m, name(document, m))),
                stock: props.stock.map(|(s, _)| (s, name(document, s))),
                supply,
                stack: stacks.get(&part).cloned(),
            });
        }
        rows.sort_by(|a, b| (a.supply, &a.name, a.part).cmp(&(b.supply, &b.name, b.part)));
        Ok(Bom {
            revision: self.snapshot().stamp.revision,
            source: ContentHash::document(document).map_err(|e| error(None, e))?,
            rows,
            excluded,
        })
    }
}

/// A table of text cells produced from authoritative records.
#[derive(Clone, Debug, PartialEq)]
pub struct Schedule {
    pub kind: &'static str,
    pub title: String,
    pub revision: u64,
    pub source: ContentHash,
    /// Units and inclusion rules, written into CSV headers.
    pub notes: Vec<String>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}
fn mm(v: f64) -> String {
    let r = (v * 1000.).round() / 1000.;
    if r == r.trunc() {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}
fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(['=', '+', '-', '@']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}
impl Schedule {
    /// RFC 4180 CSV with `#` comment lines for kind, revision, source and
    /// notes. Cells starting with formula characters are quoted.
    pub fn csv(&self) -> String {
        let mut out = format!(
            "# MultiCAD {} schedule\n# revision {}\n# source {}\n",
            self.kind, self.revision, self.source
        );
        for n in &self.notes {
            out.push_str(&format!("# {n}\n"));
        }
        let line = |cells: &[String]| {
            cells
                .iter()
                .map(|c| csv_cell(c))
                .collect::<Vec<_>>()
                .join(",")
                + "\n"
        };
        out.push_str(&line(&self.columns));
        for r in &self.rows {
            out.push_str(&line(r));
        }
        out
    }
    /// A drawing table annotation with the same cells. Column widths are
    /// fitted to the text at 2 mm per character, at least 12 mm.
    pub fn table(&self, position_mm: [f64; 2]) -> AnnotationKind {
        let widths_mm = (0..self.columns.len())
            .map(|i| {
                let chars = std::iter::once(&self.columns[i])
                    .chain(self.rows.iter().filter_map(|r| r.get(i)))
                    .map(|s| s.chars().count())
                    .max()
                    .unwrap_or(1);
                (chars as f64 * 2. + 3.).max(12.)
            })
            .collect();
        AnnotationKind::Table {
            position_mm,
            title: format!("{} (rev {})", self.title, self.revision),
            columns: self.columns.clone(),
            rows: self.rows.clone(),
            widths_mm,
            row_height_mm: 5.,
        }
    }
    /// Whether a placed table still shows this schedule's cells. A table
    /// from an older revision with different content is stale.
    pub fn check_table(&self, table: &AnnotationKind) -> Result<(), String> {
        let AnnotationKind::Table {
            columns,
            rows,
            title,
            ..
        } = table
        else {
            return Err("Not a table".into());
        };
        if *columns != self.columns || *rows != self.rows {
            return Err(format!(
                "The table \"{title}\" is stale: the {} schedule at revision {} differs",
                self.kind, self.revision
            ));
        }
        Ok(())
    }
}

/// The BOM as a schedule.
pub fn bom_schedule(bom: &Bom) -> Schedule {
    Schedule {
        kind: "bom",
        title: "Bill of materials".into(),
        revision: bom.revision,
        source: bom.source,
        notes: vec![
            "Quantities count physical Part occurrences; assemblies are not rows".into(),
            "Stack layers are listed individually with their stack".into(),
            format!("{} Parts excluded by inclusion rules", bom.excluded.len()),
        ],
        columns: [
            "Item", "Part", "Qty", "Material", "Stock", "Supply", "Stack",
        ]
        .map(String::from)
        .to_vec(),
        rows: bom
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                vec![
                    (i + 1).to_string(),
                    r.name.clone(),
                    r.quantity.to_string(),
                    r.material.as_ref().map(|m| m.1.clone()).unwrap_or_default(),
                    r.stock.as_ref().map(|s| s.1.clone()).unwrap_or_default(),
                    r.supply.label().into(),
                    r.stack.as_ref().map(|s| s.1.clone()).unwrap_or_default(),
                ]
            })
            .collect(),
    }
}
fn role(role: Option<PanelRole>) -> String {
    role.map(|r| r.label()).unwrap_or_default()
}
/// The panel cut list: finished and core sizes along and across the grain.
pub fn cut_list_schedule(
    document: &Document,
    revision: u64,
    records: &[CutRecord],
) -> Result<Schedule, ModelError> {
    Ok(Schedule {
        kind: "panel_cut_list",
        title: "Panel cut list".into(),
        revision,
        source: ContentHash::document(document).map_err(|e| error(None, e))?,
        notes: vec![
            "Millimeters; length runs along the grain (the longer side when grain is free)".into(),
            "Core = finished - band thickness + band allowance per banded edge + oversize".into(),
        ],
        columns: [
            "Panel",
            "Role",
            "Stock",
            "T",
            "Finished L",
            "Finished W",
            "Core L",
            "Core W",
            "Grain",
            "Bands",
            "Fits sheet",
            "Stack",
            "Issues",
        ]
        .map(String::from)
        .to_vec(),
        rows: records
            .iter()
            .map(|r| {
                vec![
                    r.name.clone(),
                    role(r.role),
                    r.stock_name.clone(),
                    mm(r.cut.thickness_mm),
                    mm(r.cut.finished_mm[0]),
                    mm(r.cut.finished_mm[1]),
                    mm(r.cut.core_mm[0]),
                    mm(r.cut.core_mm[1]),
                    if r.cut.grain_fixed { "fixed" } else { "free" }.into(),
                    r.bands
                        .iter()
                        .map(|(edge, b)| format!("{edge} {}", mm(b.thickness_mm)))
                        .collect::<Vec<_>>()
                        .join("; "),
                    if r.fits_stock { "yes" } else { "no" }.into(),
                    r.stack.map(|s| name(document, s)).unwrap_or_default(),
                    r.issues.join("; "),
                ]
            })
            .collect(),
    })
}
/// Machining: one row per hole group or recess per panel.
pub fn machining_schedule(
    document: &Document,
    revision: u64,
    records: &[MachiningRecord],
) -> Result<Schedule, ModelError> {
    let mut rows = vec![];
    for r in records {
        let panel = name(document, r.part);
        for x in &r.recesses {
            rows.push(vec![
                panel.clone(),
                r.name.clone(),
                x.kind.into(),
                format!("{} × {}", mm(x.size_mm[0]), mm(x.size_mm[1])),
                mm(x.depth_mm),
                String::new(),
            ]);
        }
        let mut holes: BTreeMap<(String, String), usize> = BTreeMap::new();
        for h in &r.holes {
            let depth = if h.through {
                "through".into()
            } else {
                mm(h.depth_mm)
            };
            *holes.entry((mm(h.diameter_mm), depth)).or_default() += 1;
        }
        for ((dia, depth), count) in holes {
            rows.push(vec![
                panel.clone(),
                r.name.clone(),
                "holes".into(),
                format!("Ø{dia}"),
                depth,
                count.to_string(),
            ]);
        }
    }
    Ok(Schedule {
        kind: "machining",
        title: "Machining".into(),
        revision,
        source: ContentHash::document(document).map_err(|e| error(None, e))?,
        notes: vec!["Millimeters; modeled features, not CNC toolpaths".into()],
        columns: ["Panel", "Operation", "Kind", "Size", "Depth", "Count"]
            .map(String::from)
            .to_vec(),
        rows,
    })
}
/// The member cut list: identical cuts grouped with a quantity.
pub fn member_cut_schedule(
    document: &Document,
    revision: u64,
    groups: &[crate::CutGroup],
) -> Result<Schedule, ModelError> {
    let end = |(kind, angle): &(String, f64), face: Option<&str>| {
        if kind == "square" {
            "square".to_owned()
        } else if let Some(face) = face {
            format!("{kind} {}° long point {face}", mm(*angle))
        } else {
            format!("{kind} {}°", mm(*angle))
        }
    };
    Ok(Schedule {
        kind: "member_cut_list",
        title: "Member cut list".into(),
        revision,
        source: ContentHash::document(document).map_err(|e| error(None, e))?,
        notes: vec![
            "Millimeters; length is the overall length along the member axis, long point to long point".into(),
            "Plane-cut angles are measured from square; cope angles are branch angles".into(),
            "Long point: the section face (x = width, y = height) where an angled cut reaches farthest, for the first member listed".into(),
        ],
        columns: [
            "Mark",
            "Qty",
            "Section",
            "Length",
            "Start",
            "End",
            "Long points",
            "Members",
        ]
        .map(String::from)
        .to_vec(),
        rows: groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                vec![
                    format!("M{}", i + 1),
                    g.quantity.to_string(),
                    g.designation.clone(),
                    mm(g.length_mm),
                    end(&g.ends[0], g.long_faces[0]),
                    end(&g.ends[1], g.long_faces[1]),
                    g.long_points.unwrap_or_default().into(),
                    g.names.join("; "),
                ]
            })
            .collect(),
    })
}
/// The weld schedule in authored sequence order.
pub fn weld_schedule(
    document: &Document,
    revision: u64,
    welds: &[crate::WeldRecord],
) -> Result<Schedule, ModelError> {
    Ok(Schedule {
        kind: "weld_schedule",
        title: "Weld schedule".into(),
        revision,
        source: ContentHash::document(document).map_err(|e| error(None, e))?,
        notes: vec![
            "Millimeters; welds in authored sequence order, then unsequenced".into(),
            "Documentation only, not a welding procedure or structural calculation".into(),
        ],
        columns: [
            "Step", "Mark", "Joint", "Members", "Symbol", "Size", "Process", "Notes",
        ]
        .map(String::from)
        .to_vec(),
        rows: welds
            .iter()
            .map(|w| {
                vec![
                    w.step.map(|s| s.to_string()).unwrap_or_default(),
                    w.mark.clone(),
                    w.kind.label().into(),
                    w.member_names.join(" / "),
                    format!("{:?}", w.symbol).to_lowercase(),
                    w.size_mm.map(mm).unwrap_or_default(),
                    w.process.clone(),
                    [w.step_note.as_str(), w.notes.as_str()]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join("; "),
                ]
            })
            .collect(),
    })
}
