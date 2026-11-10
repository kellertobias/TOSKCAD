//! FDM aids: tab, bridge and support solids, and the aid inspection
//! measured from the evaluated aid, target and neighbouring Part solids.
//! A support's top is derived from the target's current exact solid: the
//! lowest target material over the footprint, less the authored gap.
use crate::{GeometryBackend, Model, ModelError, error};
use multicad_core::{
    document::{Document, DocumentError, EntityId},
    model::{self, AidKind, ResolvedAid, ResolvedProcess},
    parameters::EvaluatedParameters,
    references::Origin,
};
use multicad_kernel::{
    BRepSelection, BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame,
    Solid, SolidPairRelation,
};

/// Contact tolerance for aid inspections.
const CONTACT_MM: f64 = 1e-3;
/// Allowance tolerance for comparing measured and authored values.
const TOLERANCE_MM: f64 = 1e-4;

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn unit(a: [f64; 3]) -> [f64; 3] {
    let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    a.map(|c| c / n)
}
fn add(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    std::array::from_fn(|i| a[i] + b[i] * s)
}

/// A named box: a `length` × `width` rectangle at `corner` spanned by unit
/// `u` and `v`, extruded `height` along u × v. Side faces carry `roles` in
/// edge order (v = 0, u = length, v = width, u = 0).
#[allow(clippy::too_many_arguments)]
fn oriented_box(
    corner: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    size: [f64; 3],
    id: EntityId,
    roles: [&str; 4],
    policy: GeometryPolicy,
    backend: GeometryBackend,
) -> Result<Solid, ModelError> {
    let [l, w, h] = size;
    let corners = [[0., 0.], [l, 0.], [l, w], [0., w]];
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: corner,
            u,
            v,
        },
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: corners[i],
                b: corners[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    };
    let origins: Vec<_> = roles
        .iter()
        .map(|role| Origin {
            source: id,
            role: (*role).into(),
        })
        .collect();
    let displacement = cross(u, v).map(|c| c * h);
    match backend {
        GeometryBackend::OpenCascade => {
            Solid::extrude_curved_profile_vector_named(&profile, displacement, id, &origins, policy)
        }
        GeometryBackend::Rust => {
            Solid::rust_extrude_linear_named(&profile, displacement, 0., id, &origins, policy)
        }
    }
    .map_err(|e| error(Some(id), e))
}
fn plan_axes(angle_rad: f64) -> ([f64; 3], [f64; 3]) {
    let (s, c) = angle_rad.sin_cos();
    ([c, s, 0.], [-s, c, 0.])
}

/// Build one aid solid. `target` is the target Part's current exact solid,
/// needed to derive a support's height.
pub(crate) fn build(
    document: &Document,
    parameters: &EvaluatedParameters,
    id: EntityId,
    policy: GeometryPolicy,
    target: Option<&Solid>,
    backend: GeometryBackend,
) -> Result<Solid, ModelError> {
    let e = |e: DocumentError| error(Some(id), e);
    let aid = model::read_aid(document, id).map_err(e)?;
    match aid.resolve(parameters).map_err(e)? {
        ResolvedAid::Tab {
            origin,
            angle_rad,
            length_mm,
            width_mm,
            thickness_mm,
            overlap_mm,
        } => {
            let (u, v) = plan_axes(angle_rad);
            let corner = add(add(origin, u, -overlap_mm), v, -width_mm / 2.);
            oriented_box(
                corner,
                u,
                v,
                [overlap_mm + length_mm, width_mm, thickness_mm],
                id,
                ["tab.side_start", "tab.end", "tab.side_end", "tab.contact"],
                policy,
                backend,
            )
        }
        ResolvedAid::Bridge {
            start,
            end,
            width_mm,
            thickness_mm,
        } => {
            let d = std::array::from_fn(|i| end[i] - start[i]);
            let span = (0..3).map(|i| d[i] * d[i]).sum::<f64>().sqrt();
            let u = unit(d);
            let v = unit(cross([0., 0., 1.], u));
            let n = cross(u, v);
            let corner = add(add(start, v, -width_mm / 2.), n, -thickness_mm / 2.);
            oriented_box(
                corner,
                u,
                v,
                [span, width_mm, thickness_mm],
                id,
                [
                    "bridge.side_start",
                    "bridge.end",
                    "bridge.side_end",
                    "bridge.start",
                ],
                policy,
                backend,
            )
        }
        ResolvedAid::Support {
            origin,
            angle_rad,
            length_mm,
            width_mm,
            base_mm,
            gap_mm,
        } => {
            let target = target
                .ok_or_else(|| error(Some(id), "The support target has no current exact solid"))?;
            let bounds = target.bounds().map_err(|e| error(Some(id), e))?;
            if bounds.max[2] <= base_mm {
                return Err(error(
                    Some(id),
                    "The support target lies entirely below the support base",
                ));
            }
            let (u, v) = plan_axes(angle_rad);
            let corner = [origin[0], origin[1], base_mm];
            let probe = oriented_box(
                corner,
                u,
                v,
                [length_mm, width_mm, bounds.max[2] - base_mm + 1.],
                id,
                ["probe.0", "probe.1", "probe.2", "probe.3"],
                policy,
                backend,
            )?;
            let exact_policy = GeometryPolicy {
                boolean_fuzzy_mm: 0.,
                ..policy
            };
            let intersection = match backend {
                GeometryBackend::OpenCascade => target
                    .boolean(&probe, BooleanOperation::Intersect, exact_policy)
                    .and_then(|result| result.as_ref().map(Solid::bounds).transpose()),
                // The clipped material gives the support height even when the
                // lowest point of the target lies outside the footprint.
                GeometryBackend::Rust => target
                    .rust_planar_intersection_metrics(&probe, exact_policy)
                    .map(|result| result.map(|(_, bounds)| bounds)),
            };
            let lowest = intersection
                .map_err(|e| error(Some(id), e))?
                .ok_or_else(|| {
                    error(
                        Some(id),
                        "The support footprint is not under any target material",
                    )
                })?
                .min[2];
            let height = lowest - gap_mm - base_mm;
            if height < policy.minimum_feature_mm.max(TOLERANCE_MM) {
                return Err(error(
                    Some(id),
                    format!(
                        "No room for the support: the target reaches down to {lowest:.3} mm over the footprint"
                    ),
                ));
            }
            oriented_box(
                corner,
                u,
                v,
                [length_mm, width_mm, height],
                id,
                [
                    "support.side_start",
                    "support.end",
                    "support.side_end",
                    "support.start",
                ],
                policy,
                backend,
            )
        }
    }
}

/// How an aid relates to one Part, measured from exact solids.
#[derive(Clone, Debug, PartialEq)]
pub struct AidContact {
    pub part: EntityId,
    pub relation: SolidPairRelation,
    pub distance_mm: f64,
    pub common_volume_mm3: f64,
}
/// One aid as evaluated: its measured size and its contacts, with review
/// findings against its process allowances.
#[derive(Clone, Debug, PartialEq)]
pub struct AidInspection {
    pub feature: EntityId,
    pub part: EntityId,
    pub name: String,
    pub kind: AidKind,
    pub target: AidContact,
    pub other: Option<AidContact>,
    pub process: Option<ResolvedProcess>,
    /// World bounds of the aid Part's current solid.
    pub bounds_mm: [[f64; 3]; 2],
    pub volume_mm3: f64,
    /// Parts other than the target and bridge partner closer than the fit
    /// clearance (or touching, without a process), with their distance.
    pub crowded: Vec<(EntityId, f64)>,
    pub issues: Vec<String>,
    /// The findings that make the aid's placement invalid: it misses or is
    /// buried in what it serves, or it collides with or crowds a neighbour.
    /// Each is also listed in `issues`. The window refuses to Apply an aid
    /// with any of them; allowance findings stay advisory.
    pub contact_issues: Vec<String>,
    /// A tab's exact outline contact near its authored contact point, found
    /// along its direction; the window offers it as a snap.
    pub exact_contact: Option<TabContact>,
}

/// Where a breakaway tab meets a finished Part, derived from the Part's
/// exact solid rather than from display triangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabContact {
    /// The nearest outline point, at the Part's lowest height: the tab's
    /// contact point.
    pub point_mm: [f64; 3],
    /// Unit outward direction in the build plane.
    pub outward: [f64; 2],
    /// The outward direction about +Z in degrees, from +X.
    pub angle_deg: f64,
    /// How far the probe point was from the outline.
    pub probe_distance_mm: f64,
}

/// Side of the probe cube used to find an outline point.
const PROBE_MM: f64 = 1e-4;
/// A probe on the hint side clears display chords of curved faces.
const PROBE_OFFSET_MM: f64 = 1.;

impl Model {
    /// Find a tab contact on `target` near a world point. The point is
    /// moved to just above the Part's lowest height, `outward_hint` (a
    /// build-plane direction) moves it off the surface, and the exact
    /// minimum distance to the Part gives the outline point and its outward
    /// direction, to within the 0.1 µm probe. A point inside the outline is
    /// refused.
    pub fn fdm_tab_contact(
        &self,
        target: EntityId,
        near_mm: [f64; 3],
        outward_hint: Option<[f64; 2]>,
    ) -> Result<TabContact, ModelError> {
        let solid = self
            .part_solid(target)
            .ok_or_else(|| error(Some(target), "The Part has no evaluated solid"))?;
        let b = solid.bounds().map_err(|e| error(Some(target), e))?;
        if near_mm.iter().any(|v| !v.is_finite()) {
            return Err(error(Some(target), "The contact point is not finite"));
        }
        let base = b.min[2];
        let z = base + ((b.max[2] - base) * 0.5).min(0.05);
        let mut xy = [near_mm[0], near_mm[1]];
        if let Some(h) = outward_hint {
            let n = h[0].hypot(h[1]);
            if n.is_finite() && n > 1e-9 {
                xy = [
                    xy[0] + h[0] / n * PROBE_OFFSET_MM,
                    xy[1] + h[1] / n * PROBE_OFFSET_MM,
                ];
            }
        }
        let half = PROBE_MM / 2.;
        let backend = if solid.uses_rust_geometry() {
            GeometryBackend::Rust
        } else {
            GeometryBackend::OpenCascade
        };
        let probe = oriented_box(
            [xy[0] - half, xy[1] - half, z - half],
            [1., 0., 0.],
            [0., 1., 0.],
            [PROBE_MM; 3],
            target,
            ["probe.a", "probe.b", "probe.c", "probe.d"],
            self.policy,
            backend,
        )?;
        let d = solid
            .minimum_distance(BRepSelection::Whole, &probe, BRepSelection::Vertex(0))
            .map_err(|e| error(Some(target), e))?;
        if d.inner_solution || d.distance_mm <= CONTACT_MM {
            return Err(error(
                Some(target),
                "The point is inside the part's outline at its base; pick a side face",
            ));
        }
        let w = d.witnesses[0];
        // Measured to one probe vertex, the witness segment is normal to
        // the part at its outline point.
        let along = [w.b_mm[0] - w.a_mm[0], w.b_mm[1] - w.a_mm[1]];
        let length = along[0].hypot(along[1]);
        if length < d.distance_mm * 0.5 {
            return Err(error(
                Some(target),
                "No side of the part faces the point at its base; pick a side face",
            ));
        }
        let outward = [along[0] / length, along[1] / length];
        Ok(TabContact {
            point_mm: [w.a_mm[0], w.a_mm[1], base],
            outward,
            angle_deg: outward[1].atan2(outward[0]).to_degrees(),
            probe_distance_mm: d.distance_mm,
        })
    }
    /// The Part's current exact solid. A Part that is only a reusable
    /// Assembly component definition has no world body, so it is measured
    /// in its definition frame, the frame its aids are built in.
    fn part_solid(&self, part: EntityId) -> Option<&Solid> {
        let snapshot = self.snapshot();
        let head = snapshot.bodies.get(&part).or_else(|| {
            (!snapshot.consumed_bodies.contains(&part))
                .then(|| snapshot.body_sources.get(&part))
                .flatten()
        })?;
        self.solid(*head)
    }
    fn contact(&self, aid: &Solid, part: EntityId) -> Result<Option<AidContact>, ModelError> {
        let Some(solid) = self.part_solid(part) else {
            return Ok(None);
        };
        let pair = aid
            .inspect_pair(solid, CONTACT_MM, self.policy)
            .map_err(|e| error(Some(part), e))?;
        Ok(Some(AidContact {
            part,
            relation: pair.relation,
            distance_mm: pair.minimum_distance.distance_mm,
            common_volume_mm3: pair.common_volume_mm3,
        }))
    }
    /// Inspect every evaluated FDM aid against its target, bridge partner,
    /// neighbouring Parts and process allowances.
    pub fn fdm_aids(&self, document: &Document) -> Result<Vec<AidInspection>, ModelError> {
        let parameters =
            multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
        let aid_parts = model::aid_parts(document);
        let mut out = vec![];
        for entity in model::aids(document) {
            let id = entity.id;
            let e = |e: DocumentError| error(Some(id), e);
            let aid = model::read_aid(document, id).map_err(e)?;
            let part = aid.output.body;
            let Some(solid) = self.part_solid(part) else {
                continue;
            };
            let name = document
                .get(part)
                .map_or_else(|| part.to_string(), |p| p.name.clone());
            let label = |p: EntityId| {
                document
                    .get(p)
                    .map_or_else(|| p.to_string(), |e| e.name.clone())
            };
            let resolved = aid.resolve(&parameters).map_err(e)?;
            let process = aid
                .process
                .map(|p| model::read_process(document, p).and_then(|d| d.resolve(&parameters)))
                .transpose()
                .map_err(e)?;
            let b = solid.bounds().map_err(|e| error(Some(id), e))?;
            let volume = solid.volume_mm3().map_err(|e| error(Some(id), e))?;
            let mut issues = vec![];
            let target = match self.contact(solid, aid.target)? {
                Some(c) => c,
                None => {
                    issues.push(format!("{} has no evaluated solid", label(aid.target)));
                    AidContact {
                        part: aid.target,
                        relation: SolidPairRelation::Separated,
                        distance_mm: f64::INFINITY,
                        common_volume_mm3: 0.,
                    }
                }
            };
            let other = match aid.shape.other() {
                Some(o) => {
                    let c = self.contact(solid, o)?;
                    if c.is_none() {
                        issues.push(format!("{} has no evaluated solid", label(o)));
                    }
                    c
                }
                None => None,
            };
            let exact_contact = match resolved {
                ResolvedAid::Tab {
                    origin, angle_rad, ..
                } => self
                    .fdm_tab_contact(aid.target, origin, Some([angle_rad.cos(), angle_rad.sin()]))
                    .ok(),
                _ => None,
            };
            let touches = |c: &AidContact| c.relation != SolidPairRelation::Separated;
            match resolved {
                ResolvedAid::Tab { .. } => {
                    if !touches(&target) {
                        issues.push(format!(
                            "The tab does not touch {}: it is {:.3} mm away",
                            label(aid.target),
                            target.distance_mm
                        ));
                    }
                }
                ResolvedAid::Bridge { .. } => {
                    if !touches(&target) {
                        issues.push(format!(
                            "The bridge does not reach {}: it is {:.3} mm away",
                            label(aid.target),
                            target.distance_mm
                        ));
                    }
                    if let Some(o) = &other
                        && !touches(o)
                    {
                        issues.push(format!(
                            "The bridge does not reach {}: it is {:.3} mm away",
                            label(o.part),
                            o.distance_mm
                        ));
                    }
                    let buried = target.common_volume_mm3
                        + other.as_ref().map_or(0., |o| o.common_volume_mm3);
                    if buried >= volume * (1. - 1e-6) {
                        issues.push("The bridge lies entirely inside the Parts it ties".into());
                    }
                }
                ResolvedAid::Support { gap_mm, .. } => {
                    if target.relation == SolidPairRelation::Overlapping {
                        issues.push(format!(
                            "The support overlaps {} by {:.3} mm³",
                            label(aid.target),
                            target.common_volume_mm3
                        ));
                    } else if target.distance_mm + TOLERANCE_MM < gap_mm {
                        issues.push(format!(
                            "The support comes {:.3} mm from {}, closer than its {gap_mm} mm gap",
                            target.distance_mm,
                            label(aid.target)
                        ));
                    }
                    if let Some(p) = &process
                        && target.distance_mm + TOLERANCE_MM < p.support_gap_mm
                    {
                        issues.push(format!(
                            "The support gap {:.3} mm is below the process support gap {} mm",
                            target.distance_mm, p.support_gap_mm
                        ));
                    }
                }
            }
            // Everything found so far concerns placement and contact.
            let mut contact_issues = issues.clone();
            if let Some(p) = &process {
                if let Some(t) = resolved.thickness_mm() {
                    if t + TOLERANCE_MM < p.min_thickness_mm() {
                        issues.push(format!(
                            "Thickness {t} mm is below {} layers of {} mm",
                            p.min_layers, p.layer_height_mm
                        ));
                    }
                    let layers = t / p.layer_height_mm;
                    if (layers - layers.round()).abs() * p.layer_height_mm > TOLERANCE_MM {
                        issues.push(format!(
                            "Thickness {t} mm is not a whole number of {} mm layers",
                            p.layer_height_mm
                        ));
                    }
                }
                let w = resolved.width_mm();
                if w + TOLERANCE_MM < p.min_width_mm() {
                    issues.push(format!(
                        "Width {w} mm is below {} lines of {} mm",
                        p.min_lines, p.line_width_mm
                    ));
                }
            }
            // Fit: every other finished Part must keep the clearance. Aid
            // Parts are not checked against each other.
            let clearance = process.map_or(0., |p| p.fit_clearance_mm);
            let mut crowded = vec![];
            for (&candidate, head) in &self.snapshot().bodies {
                if candidate == part
                    || candidate == aid.target
                    || Some(candidate) == aid.shape.other()
                    || aid_parts.contains(&candidate)
                    || self.snapshot().consumed_bodies.contains(&candidate)
                {
                    continue;
                }
                let Some(other_solid) = self.solid(*head) else {
                    continue;
                };
                let ob = other_solid
                    .bounds()
                    .map_err(|e| error(Some(candidate), e))?;
                let reach = clearance + CONTACT_MM;
                if (0..3).any(|i| ob.min[i] > b.max[i] + reach || b.min[i] > ob.max[i] + reach) {
                    continue;
                }
                let pair = solid
                    .inspect_pair(other_solid, CONTACT_MM, self.policy)
                    .map_err(|e| error(Some(candidate), e))?;
                let d = pair.minimum_distance.distance_mm;
                if pair.relation != SolidPairRelation::Separated || d + TOLERANCE_MM < clearance {
                    let finding = format!(
                        "{} is {d:.3} mm from {}, inside the {clearance} mm fit clearance",
                        name,
                        label(candidate)
                    );
                    issues.push(finding.clone());
                    contact_issues.push(finding);
                    crowded.push((candidate, d));
                }
            }
            out.push(AidInspection {
                feature: id,
                part,
                name,
                kind: aid.shape.kind(),
                target,
                other,
                process,
                bounds_mm: [b.min, b.max],
                volume_mm3: volume,
                crowded,
                issues,
                contact_issues,
                exact_contact,
            });
        }
        Ok(out)
    }
}
