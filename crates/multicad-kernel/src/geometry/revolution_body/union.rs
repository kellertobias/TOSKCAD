//! The connected union of coaxial cylinders is the maximum radius at
//! each axial level. Build its exact stepped meridian in one pass.
use super::*;
use std::{collections::BTreeMap, f64::consts::TAU};

const UNION: &str = "coaxial cylinder union";
const MAX_CYLINDERS: usize = 129;

struct Operand {
    span: [f64; 2],
    radius: f64,
    caps: [Ancestry; 2],
    wall: Vec<Origin>,
}

struct Step {
    span: [f64; 2],
    radius: f64,
    origins: BTreeSet<Origin>,
}

#[derive(Default)]
struct Ancestry {
    origins: BTreeSet<Origin>,
    frames: BTreeMap<Origin, ReferenceFrame>,
}

impl Operand {
    fn new(solid: &Solid, reference: PlaneFrame, policy: GeometryPolicy) -> Option<Self> {
        if solid.policy != policy {
            return None;
        }
        let (cylinder, frame) = solid.axial_cylinder_frame()?;
        let axis = reference.normal();
        let normal = frame.normal();
        let forward = if normal == axis {
            true
        } else if normal == axis.map(|c| -c) {
            false
        } else {
            return None;
        };
        let delta = difference(frame.origin, reference.origin);
        if cross(delta, axis) != [0.; 3] {
            return None;
        }
        let start = dot(delta, unit(axis));
        let travel = cylinder
            .height()
            .as_f64()
            .copysign(if forward { 1. } else { -1. });
        let end = start + travel;
        let feature = EntityId::from_uuid_bits(cylinder.feature());
        let origin = |role: String| Origin {
            source: feature,
            role,
        };
        let sketch = solid.cylinder_sketch();
        let cap = |end: bool| {
            let role = origin(if end { "cap.end" } else { "cap.start" }.into());
            let center = position(
                frame,
                [0., 0., if end { cylinder.height().as_f64() } else { 0. }],
            );
            let v = if sketch.is_some_and(|sketch| sketch.reverse) {
                frame.v.map(|x| -x)
            } else {
                frame.v
            };
            let anchor = sketch.map_or(center, |sketch| {
                let [x, y] = sketch.center_uv;
                std::array::from_fn(|i| center[i] - x * frame.u[i] - y * v[i])
            });
            let origins = std::iter::once(role)
                .chain(solid.rust_origins.iter().cloned())
                .collect::<BTreeSet<_>>();
            let support = ReferenceFrame {
                origin_mm: anchor,
                u: frame.u,
                v,
            };
            let frames = origins
                .iter()
                .cloned()
                .map(|origin| (origin, support))
                .collect();
            Ancestry { origins, frames }
        };
        let mut wall = solid.rust_origins.clone();
        if let Some(sketch) = sketch {
            wall.push(origin(format!(
                "side/{}/{}",
                sketch.origin.source, sketch.origin.role
            )));
            wall.push(sketch.origin.clone());
        } else {
            wall.push(origin("side.cylinder".into()));
        }
        Some(Self {
            span: [start.min(end), start.max(end)],
            radius: cylinder.circle().radius().as_f64(),
            caps: [cap(!forward), cap(forward)],
            wall,
        })
    }
}

impl Solid {
    /// Recognize an unfuzzed union of cylinders on one exact line, in either
    /// axial direction. Other shapes remain eligible for their own providers.
    /// A stepped result is analytic and supports mesh and references. STEP
    /// currently handles only the uniform-radius result of this family.
    pub fn union_coaxial_cylinders(
        solids: &[&Self],
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Option<Result<Self, GeometryError>> {
        let [first, _, ..] = solids else {
            return None;
        };
        if policy.boolean_fuzzy_mm != 0. {
            return None;
        }
        let (_, frame) = first.axial_cylinder_frame()?;
        if solids.len() > MAX_CYLINDERS {
            return Some(Err(error(
                UNION,
                GeometryErrorKind::Limit,
                "Coaxial union exceeds the cylinder limit",
            )));
        }
        let operands: Vec<_> = solids
            .iter()
            .map(|solid| Operand::new(solid, frame, policy))
            .collect::<Option<_>>()?;
        Some(build(frame, &operands, feature, policy))
    }
}

fn build(
    frame: PlaneFrame,
    operands: &[Operand],
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, GeometryError> {
    let mut levels: Vec<_> = operands.iter().flat_map(|cylinder| cylinder.span).collect();
    levels.sort_by(f64::total_cmp);
    levels.dedup();
    let mut steps: Vec<Step> = Vec::with_capacity(levels.len());
    for pair in levels.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        let covers = |cylinder: &&Operand| cylinder.span[0] <= lo && cylinder.span[1] >= hi;
        let radius = operands
            .iter()
            .filter(covers)
            .map(|cylinder| cylinder.radius)
            .reduce(f64::max)
            .ok_or_else(|| {
                error(
                    UNION,
                    GeometryErrorKind::InvalidResult,
                    "Boolean produced disconnected fragments; coaxial cylinder spans have a gap",
                )
            })?;
        let origins = operands
            .iter()
            .filter(covers)
            .filter(|cylinder| cylinder.radius == radius)
            .flat_map(|cylinder| cylinder.wall.iter().cloned());
        if let Some(step) = steps.last_mut().filter(|step| step.radius == radius) {
            step.span[1] = hi;
            step.origins.extend(origins);
        } else {
            steps.push(Step {
                span: [lo, hi],
                radius,
                origins: origins.collect(),
            });
        }
    }
    let first = steps
        .first()
        .ok_or_else(|| input("Coaxial cylinder union requires positive material"))?;
    let last = steps.last().expect("first step exists");
    let cap_ancestry = |level, radius, upper: bool| {
        let end = usize::from(upper);
        let mut ancestry = Ancestry::default();
        for operand in operands
            .iter()
            .filter(|operand| operand.span[end] == level && operand.radius == radius)
        {
            ancestry
                .origins
                .extend(operand.caps[end].origins.iter().cloned());
            ancestry.frames.extend(
                operand.caps[end]
                    .frames
                    .iter()
                    .map(|(origin, frame)| (origin.clone(), *frame)),
            );
        }
        ancestry
    };
    let mut path = vec![(
        [0., first.span[0]],
        cap_ancestry(first.span[0], first.radius, false),
    )];
    for (i, step) in steps.iter().enumerate() {
        let [lo, hi] = step.span;
        let radius = step.radius;
        path.push((
            [radius, lo],
            Ancestry {
                origins: step.origins.clone(),
                ..Default::default()
            },
        ));
        let next = steps.get(i + 1).map_or(0., |step| step.radius);
        path.push((
            [radius, hi],
            cap_ancestry(hi, radius.max(next), radius > next),
        ));
    }
    path.push(([0., last.span[1]], Ancestry::default()));

    let origins: Vec<_> = (0..path.len())
        .map(|i| Origin {
            source: feature,
            role: format!("coaxial.profile.{i}"),
        })
        .collect();
    let axis = unit(frame.normal());
    let profile = CurvedProfile {
        frame: PlaneFrame { v: axis, ..frame },
        outer: path
            .iter()
            .zip(path.iter().cycle().skip(1))
            .map(|(a, b)| CurvedEdge2d::Line { a: a.0, b: b.0 })
            .collect(),
        holes: vec![],
    };
    let ancestry: BTreeMap<_, _> = origins
        .iter()
        .cloned()
        .zip(path.into_iter().map(|(_, origins)| origins))
        .collect();
    let mut result = Solid::rust_revolve_curved_profile_named(
        &profile,
        RevolutionAxis {
            origin_mm: frame.origin,
            direction: axis,
        },
        TAU,
        feature,
        &origins,
        policy,
    )?;
    let SolidBody::Revolved(body, frame) = &mut result.body else {
        unreachable!("Rust revolution provider")
    };
    let axes = [frame.u, frame.v, frame.normal()];
    let direction = |vector| axes.map(|axis| dot(axis, vector));
    let body = Rc::get_mut(body).expect("new revolution is uniquely owned");
    for segment in body.segments.iter_mut().flatten() {
        let ancestry = &ancestry[&segment.origin];
        segment.inherited = ancestry.origins.iter().cloned().collect();
        segment.inherited_frames = ancestry
            .frames
            .iter()
            .map(|(origin, anchor)| AnchoredFrame {
                origin: origin.clone(),
                frame: ReferenceFrame {
                    origin_mm: local(*frame, anchor.origin_mm),
                    u: direction(anchor.u),
                    v: direction(anchor.v),
                },
            })
            .collect();
    }
    Ok(result)
}
