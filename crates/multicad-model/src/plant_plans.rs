//! Authored, transparent planting linework. Physical sections and elevations
//! continue to consume the actual 3D envelopes.
use crate::{Model, ModelError, compiler::error, drawings::DrawingBody};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::*,
    plants,
    sketch::{self, Geometry},
    symbols,
};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::TAU;
pub(crate) struct PlantPlan {
    pub vectors: Vec<DrawingVector>,
    pub labels: Vec<SourceLabel>,
}
pub(crate) fn plans(
    document: &Document,
    model: &Model,
    revision: u64,
    view: &DrawingView,
    bodies: &[DrawingBody<'_>],
) -> Result<BTreeMap<EntityId, PlantPlan>, ModelError> {
    if view.kind != ViewKind::Projection
        || view.frame.normal()[2] < 1. - 1e-8
        || !matches!(view.source, ViewSource::Model { .. })
    {
        return Ok(BTreeMap::new());
    }
    let selected: BTreeSet<_> = bodies.iter().map(|b| b.identity).collect();
    let mut records = vec![];
    for entity in document.entities() {
        if let EntityData::Extension { definition, .. } = &entity.data
            && definition.type_id == symbols::INSTANCE_TYPE
        {
            let symbol = symbols::read_instance(document, entity.id)
                .map_err(|e| error(Some(entity.id), e))?;
            if selected.contains(&symbol.instance) {
                records.push((entity.id, symbol.instance));
            }
        }
    }
    if records.is_empty() {
        return Ok(BTreeMap::new());
    }
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
    let resolver = plants::Resolver::new(document, &parameters).map_err(|e| error(None, e))?;
    let mut output = BTreeMap::new();
    for (record, instance) in records {
        let Some(plant) = resolver
            .resolve(record)
            .map_err(|e| error(Some(instance), e))?
        else {
            continue;
        };
        let pose = model
            .snapshot()
            .occurrences
            .get(&instance)
            .ok_or_else(|| error(Some(instance), "Plant has no current occurrence pose"))?
            .pose;
        let evaluated = multicad_solver::evaluate_document_sketch_with_parameters(
            document,
            revision,
            plant.plan,
            Some(&model.snapshot().references),
            &parameters,
        )
        .map_err(|e| error(Some(instance), e))?;
        // A tilted plant remains a physical projection. A planar authored symbol is
        // only substituted when its plane is parallel to the drawing plane.
        if sketch::dot(pose.vector(evaluated.frame.n), view.frame.normal()) < 1. - 1e-8 {
            continue;
        }
        let entry = output.entry(instance).or_insert_with(|| PlantPlan {
            vectors: vec![],
            labels: vec![],
        });
        let vectors = &mut entry.vectors;
        if !plant.plan_visible {
            continue;
        }
        let origin = project_origin(view, pose, &evaluated);
        if let Some(text) = plant.label {
            entry.labels.push(SourceLabel {
                source: instance,
                at_mm: origin,
                text,
            });
        }
        vectors.extend(project_sketch(view, instance, pose, &evaluated, 1.)?);
    }
    Ok(output)
}

fn project_origin(
    view: &DrawingView,
    pose: multicad_core::model::RigidPose,
    evaluated: &multicad_solver::EvaluatedSketch,
) -> [f64; 2] {
    let world = pose.point(evaluated.frame.point([0., 0.]));
    let delta = std::array::from_fn(|i| world[i] - view.frame.origin_mm[i]);
    [
        sketch::dot(delta, view.frame.u),
        sketch::dot(delta, view.frame.v),
    ]
}
/// Shared exact authored geometry path for true-size footprints and fixed-paper marks.
pub(crate) fn project_sketch(
    view: &DrawingView,
    instance: EntityId,
    pose: multicad_core::model::RigidPose,
    evaluated: &multicad_solver::EvaluatedSketch,
    model_scale: f64,
) -> Result<Vec<DrawingVector>, ModelError> {
    let mut vectors = vec![];
    let project = |p: [f64; 2]| {
        let world = pose.point(evaluated.frame.point(p.map(|v| v * model_scale)));
        let delta = std::array::from_fn(|i| world[i] - view.frame.origin_mm[i]);
        [
            sketch::dot(delta, view.frame.u),
            sketch::dot(delta, view.frame.v),
        ]
    };
    let origin = project([0., 0.]);
    let axis = |p: [f64; 2]| {
        let p = project(p);
        [p[0] - origin[0], p[1] - origin[1]]
    };
    let mut line = |curve, preview_mm| {
        vectors.push(DrawingVector {
            source: instance,
            material_part: instance,
            role: VectorRole::Visible,
            curve,
            preview_mm,
            line_width_mm: 0.18,
        })
    };
    let circle = |center: [f64; 2], radius: f64, start: f64, end: f64| {
        let tolerance = view.paper_chord_tolerance_mm * view.scale.denominator as f64
            / view.scale.numerator as f64;
        let angle = 2. * (1. - (tolerance / (radius * model_scale)).min(1.)).acos();
        let count = ((end - start).abs() / angle.max(1e-12)).ceil() as usize;
        if count > 8192 {
            return Err(error(
                Some(instance),
                "Plant plan exceeds its curve sampling budget at this paper tolerance/scale",
            ));
        }
        let count = count.max(16).div_ceil(4) * 4;
        let preview = (0..=count)
            .map(|i| {
                let t = start + (end - start) * i as f64 / count as f64;
                project([center[0] + radius * t.cos(), center[1] + radius * t.sin()])
            })
            .collect();
        Ok((
            VectorCurve::Ellipse {
                center: project(center),
                u: axis([1. / model_scale, 0.]),
                v: axis([0., 1. / model_scale]),
                major_mm: radius * model_scale,
                minor_mm: radius * model_scale,
                start_rad: start,
                end_rad: end,
            },
            preview,
        ))
    };
    for geometry in evaluated.geometry().map_err(|e| error(Some(instance), e))? {
        match geometry {
            Geometry::Line { a, b, .. } => {
                let (a, b) = (project(a), project(b));
                line(VectorCurve::Line { a, b }, vec![a, b]);
            }
            Geometry::Circle {
                center, radius_mm, ..
            } => {
                let (c, p) = circle(center, radius_mm, 0., TAU)?;
                line(c, p);
            }
            Geometry::Arc {
                center,
                a,
                b,
                clockwise,
                ..
            } => {
                let start = (a[1] - center[1]).atan2(a[0] - center[0]);
                let mut end = (b[1] - center[1]).atan2(b[0] - center[0]);
                if clockwise {
                    while end >= start {
                        end -= TAU;
                    }
                } else {
                    while end <= start {
                        end += TAU;
                    }
                }
                let radius = (a[0] - center[0]).hypot(a[1] - center[1]);
                let (c, p) = circle(center, radius, start, end)?;
                line(c, p);
            }
            Geometry::Rectangle { a, b, .. } => {
                let points = [a, [b[0], a[1]], b, [a[0], b[1]], a];
                for pair in points.windows(2) {
                    let (a, b) = (project(pair[0]), project(pair[1]));
                    line(VectorCurve::Line { a, b }, vec![a, b]);
                }
            }
            Geometry::BSpline { poles, basis, .. } => {
                let preview = basis
                    .display_segments(&poles)
                    .map_err(|e| error(Some(instance), e))?
                    .into_iter()
                    .enumerate()
                    .flat_map(|(i, p)| {
                        if i == 0 {
                            vec![project(p[0]), project(p[1])]
                        } else {
                            vec![project(p[1])]
                        }
                    })
                    .collect();
                let weights = basis
                    .weights
                    .clone()
                    .unwrap_or_else(|| vec![1.; poles.len()]);
                line(
                    VectorCurve::BSpline {
                        degree: basis.degree,
                        periodic: false,
                        poles: poles.into_iter().map(project).collect(),
                        weights,
                        first: basis.knots[0],
                        last: *basis.knots.last().expect("validated spline"),
                        knots: basis.knots,
                        multiplicities: basis.multiplicities,
                    },
                    preview,
                );
            }
        }
    }
    Ok(vectors)
}
