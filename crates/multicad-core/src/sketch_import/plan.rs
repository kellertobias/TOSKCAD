//! Placement of parsed artwork into a sketch frame and conversion into
//! canonical curves. Placement is a similarity, so circular arcs stay
//! analytic. Coincident curve endpoints become one shared point identity;
//! existing sketch geometry is never merged or modified.
use super::{
    Artwork, Diagnostic, Format, ImportError, UnitChoice, fail,
    geometry::{Affine, Bounds, P, Segment, transform},
};
use crate::{
    constraint_sketch::{Curve, CurveKind, SplineBasis, source::Source},
    document::EntityId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const DEFAULT_WELD_TOLERANCE_MM: f64 = 1e-4;
const SKETCH_POINTS: usize = 256;
const SKETCH_CURVES: usize = 512;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportOptions {
    pub units: UnitChoice,
    /// Additional uniform factor after unit conversion.
    pub scale: f64,
    /// Sketch-local position of the source origin.
    pub insertion_mm: [f64; 2],
    /// Counterclockwise rotation about the insertion point.
    pub rotation_deg: f64,
    /// `None` imports every layer or group.
    pub layers: Option<BTreeSet<String>>,
    pub weld_tolerance_mm: f64,
    pub construction: bool,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            units: UnitChoice::FromFile,
            scale: 1.,
            insertion_mm: [0., 0.],
            rotation_deg: 0.,
            layers: None,
            weld_tolerance_mm: DEFAULT_WELD_TOLERANCE_MM,
            construction: false,
        }
    }
}
impl ImportOptions {
    pub fn validate(&self) -> Result<(), ImportError> {
        if !(self.scale.is_finite() && self.scale > 1e-9 && self.scale <= 1e9) {
            return fail("Import scale must be a positive finite factor within 1e-9..1e9");
        }
        if self
            .insertion_mm
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e9)
            || !self.rotation_deg.is_finite()
            || self.rotation_deg.abs() > 1e6
        {
            return fail("Import insertion point and rotation must be finite and bounded");
        }
        if !(self.weld_tolerance_mm.is_finite() && (1e-7..=1.).contains(&self.weld_tolerance_mm)) {
            return fail("Endpoint weld tolerance must be within 1e-7..1 mm");
        }
        Ok(())
    }
    pub fn placement(&self) -> Affine {
        Affine::translate(self.insertion_mm[0], self.insertion_mm[1])
            .then_after(Affine::rotate(self.rotation_deg.to_radians()))
            .then_after(Affine::scale(self.scale, self.scale))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerSummary {
    pub name: String,
    pub paths: usize,
    pub curves: usize,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurveCounts {
    pub lines: usize,
    pub arcs: usize,
    pub circles: usize,
    pub splines: usize,
}
impl CurveCounts {
    pub fn total(&self) -> usize {
        self.lines + self.arcs + self.circles + self.splines
    }
}

/// Source-neutral summary shown before Apply. Bounds and counts are computed
/// from the exact placed curves; spline bounds use the containing control hull.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportPreview {
    pub format: Format,
    pub layers: Vec<LayerSummary>,
    pub bounds_mm: Option<[[f64; 2]; 2]>,
    pub curves: CurveCounts,
    pub closed_loops: usize,
    pub open_chains: usize,
    pub new_points: usize,
    pub skipped_degenerate: usize,
    pub unsupported: Vec<Diagnostic>,
    pub notes: Vec<String>,
    /// `Err` explains why Apply is unavailable, such as exceeded sketch limits.
    pub applicable: Result<(), String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportPlan {
    pub format: Format,
    pub options: ImportOptions,
    pub millimeters_per_unit: f64,
    pub paths: Vec<Vec<Segment>>,
    pub layers: Vec<LayerSummary>,
    pub unsupported: Vec<Diagnostic>,
    pub notes: Vec<String>,
    pub bounds: Bounds,
}

pub fn plan(artwork: &Artwork, options: &ImportOptions) -> Result<ImportPlan, ImportError> {
    options.validate()?;
    if let Some(layers) = &options.layers
        && let Some(missing) = layers.iter().find(|l| !artwork.layers.contains(l))
    {
        return fail(format!(
            "Selected layer {missing:?} is not present in the file"
        ));
    }
    let placement = options.placement();
    let selected = |layer: &str| options.layers.as_ref().is_none_or(|s| s.contains(layer));
    let mut summaries: Vec<LayerSummary> = artwork
        .layers
        .iter()
        .map(|name| LayerSummary {
            name: name.clone(),
            paths: 0,
            curves: 0,
            selected: selected(name),
        })
        .collect();
    let mut paths = Vec::new();
    let mut bounds = Bounds::default();
    for path in &artwork.paths {
        if let Some(summary) = summaries.iter_mut().find(|s| s.name == path.layer) {
            summary.paths += 1;
            summary.curves += path.segments.len();
        }
        if !selected(&path.layer) {
            continue;
        }
        let placed: Vec<_> = path
            .segments
            .iter()
            .map(|s| transform(s, &placement))
            .collect();
        placed.iter().for_each(|s| s.extend_bounds(&mut bounds));
        paths.push(placed);
    }
    if !bounds.is_empty() && bounds.min.iter().chain(&bounds.max).any(|v| v.abs() > 1e9) {
        return fail("Placed artwork extends beyond the ±1e9 mm modeling bounds");
    }
    Ok(ImportPlan {
        format: artwork.format,
        options: options.clone(),
        millimeters_per_unit: artwork.millimeters_per_unit,
        paths,
        layers: summaries,
        unsupported: artwork.unsupported.clone(),
        notes: artwork.notes.clone(),
        bounds,
    })
}

/// Result of appending a plan to a copy of a sketch source.
#[derive(Clone, Debug, PartialEq)]
pub struct Appended {
    pub source: Source,
    /// New curves in source order.
    pub curves: Vec<EntityId>,
    pub counts: CurveCounts,
    pub new_points: usize,
    pub skipped_degenerate: usize,
    pub closed_loops: usize,
    pub open_chains: usize,
}

struct Welder {
    tolerance: f64,
    grid: HashMap<(i64, i64), Vec<(P, EntityId)>>,
}
impl Welder {
    fn key(&self, p: P) -> (i64, i64) {
        (
            (p[0] / self.tolerance).floor() as i64,
            (p[1] / self.tolerance).floor() as i64,
        )
    }
    fn point(&mut self, source: &mut Source, p: P, created: &mut usize) -> EntityId {
        let (kx, ky) = self.key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(found) = self.grid.get(&(kx + dx, ky + dy)).and_then(|cell| {
                    cell.iter()
                        .find(|(q, _)| (q[0] - p[0]).hypot(q[1] - p[1]) <= self.tolerance)
                }) {
                    return found.1;
                }
            }
        }
        let id = source.add_point(p);
        *created += 1;
        self.grid.entry((kx, ky)).or_default().push((p, id));
        id
    }
}

impl ImportPlan {
    /// Appends canonical curves to a copy of `target`. The copy is validated;
    /// exceeding interactive sketch limits returns an explicit error.
    pub fn append_to(&self, target: &Source) -> Result<Appended, ImportError> {
        let mut source = target.clone();
        let mut welder = Welder {
            tolerance: self.options.weld_tolerance_mm,
            grid: HashMap::new(),
        };
        let mut new_points = 0;
        let mut curves = Vec::new();
        let mut counts = CurveCounts::default();
        let mut skipped = 0;
        let construction = self.options.construction;
        let push = |source: &mut Source, kind: CurveKind, curves: &mut Vec<EntityId>| {
            let id = EntityId::new();
            source.sketch.curves.push(Curve {
                id,
                construction,
                kind,
            });
            curves.push(id);
        };
        // Endpoint graph for loop classification; closed single curves are loops.
        let mut edges: Vec<(EntityId, EntityId)> = Vec::new();
        let mut self_closed = 0;
        for segment in self.paths.iter().flatten() {
            match segment {
                Segment::Line { a, b } => {
                    let start = welder.point(&mut source, *a, &mut new_points);
                    let end = welder.point(&mut source, *b, &mut new_points);
                    if start == end {
                        skipped += 1;
                        continue;
                    }
                    push(&mut source, CurveKind::Line { start, end }, &mut curves);
                    counts.lines += 1;
                    edges.push((start, end));
                }
                Segment::Arc { center, sweep, .. } => {
                    let (a, b) = (segment.start().expect("arc"), segment.end().expect("arc"));
                    let start = welder.point(&mut source, a, &mut new_points);
                    let end = welder.point(&mut source, b, &mut new_points);
                    if start == end {
                        skipped += 1;
                        continue;
                    }
                    let center = source.add_point(*center);
                    new_points += 1;
                    push(
                        &mut source,
                        CurveKind::Arc {
                            center,
                            start,
                            end,
                            clockwise: *sweep < 0.,
                        },
                        &mut curves,
                    );
                    counts.arcs += 1;
                    edges.push((start, end));
                }
                Segment::Circle { center, radius } => {
                    if *radius <= 1e-7 {
                        skipped += 1;
                        continue;
                    }
                    let center = source.add_point(*center);
                    new_points += 1;
                    push(
                        &mut source,
                        CurveKind::Circle {
                            center,
                            radius_mm: *radius,
                        },
                        &mut curves,
                    );
                    counts.circles += 1;
                    self_closed += 1;
                }
                Segment::Spline {
                    degree,
                    poles,
                    knots,
                    multiplicities,
                    weights,
                } => {
                    let basis = SplineBasis {
                        degree: *degree,
                        knots: knots.clone(),
                        multiplicities: multiplicities.clone(),
                        weights: weights.clone(),
                    };
                    basis
                        .validate(poles.len())
                        .map_err(|e| ImportError(format!("Imported spline is invalid: {e}")))?;
                    let n = poles.len();
                    let first = welder.point(&mut source, poles[0], &mut new_points);
                    let last = welder.point(&mut source, poles[n - 1], &mut new_points);
                    let mut ids = vec![first];
                    for p in &poles[1..n - 1] {
                        ids.push(source.add_point(*p));
                        new_points += 1;
                    }
                    ids.push(last);
                    push(
                        &mut source,
                        CurveKind::BSpline { poles: ids, basis },
                        &mut curves,
                    );
                    counts.splines += 1;
                    if first == last {
                        self_closed += 1;
                    } else {
                        edges.push((first, last));
                    }
                }
            }
        }
        let used_points = source.sketch.points.len();
        let used_curves = source.sketch.curves.len();
        if used_points > SKETCH_POINTS || used_curves > SKETCH_CURVES {
            return fail(format!(
                "Import needs {new_points} points and {} curves; the sketch allows {SKETCH_POINTS} points and {SKETCH_CURVES} curves ({} and {} already used). Select fewer layers or split the artwork across sketches",
                curves.len(),
                target.sketch.points.len(),
                target.sketch.curves.len(),
            ));
        }
        source
            .validate()
            .map_err(|e| ImportError(format!("Imported sketch source is invalid: {e}")))?;
        let (loops, open) = classify(&edges);
        Ok(Appended {
            source,
            curves,
            counts,
            new_points,
            skipped_degenerate: skipped,
            closed_loops: loops + self_closed,
            open_chains: open,
        })
    }

    pub fn preview(&self, target: &Source) -> ImportPreview {
        let appended = self.append_to(target);
        let (counts, closed, open, points, skipped) = match &appended {
            Ok(a) => (
                a.counts,
                a.closed_loops,
                a.open_chains,
                a.new_points,
                a.skipped_degenerate,
            ),
            Err(_) => (self.raw_counts(), 0, 0, 0, 0),
        };
        let applicable = match appended {
            Ok(a) if a.curves.is_empty() => {
                Err("No importable curves on the selected layers".into())
            }
            Ok(_) => Ok(()),
            Err(e) => Err(e.0),
        };
        ImportPreview {
            format: self.format,
            layers: self.layers.clone(),
            bounds_mm: (!self.bounds.is_empty()).then_some([self.bounds.min, self.bounds.max]),
            curves: counts,
            closed_loops: closed,
            open_chains: open,
            new_points: points,
            skipped_degenerate: skipped,
            unsupported: self.unsupported.clone(),
            notes: self.notes.clone(),
            applicable,
        }
    }

    fn raw_counts(&self) -> CurveCounts {
        let mut c = CurveCounts::default();
        for s in self.paths.iter().flatten() {
            match s {
                Segment::Line { .. } => c.lines += 1,
                Segment::Arc { .. } => c.arcs += 1,
                Segment::Circle { .. } => c.circles += 1,
                Segment::Spline { .. } => c.splines += 1,
            }
        }
        c
    }
}

/// Connected components of the endpoint graph: a component whose vertices all
/// have degree two is a closed loop; anything else is an open or branching chain.
fn classify(edges: &[(EntityId, EntityId)]) -> (usize, usize) {
    let mut parent: BTreeMap<EntityId, EntityId> = BTreeMap::new();
    let mut degree: BTreeMap<EntityId, usize> = BTreeMap::new();
    fn find(parent: &mut BTreeMap<EntityId, EntityId>, x: EntityId) -> EntityId {
        let p = *parent.entry(x).or_insert(x);
        if p == x {
            return x;
        }
        let root = find(parent, p);
        parent.insert(x, root);
        root
    }
    for (a, b) in edges {
        *degree.entry(*a).or_default() += 1;
        *degree.entry(*b).or_default() += 1;
        let (ra, rb) = (find(&mut parent, *a), find(&mut parent, *b));
        if ra != rb {
            parent.insert(ra, rb);
        }
    }
    let mut closed: BTreeMap<EntityId, bool> = BTreeMap::new();
    for (v, d) in degree {
        let root = find(&mut parent, v);
        let entry = closed.entry(root).or_insert(true);
        *entry &= d == 2;
    }
    let loops = closed.values().filter(|c| **c).count();
    (loops, closed.len() - loops)
}
