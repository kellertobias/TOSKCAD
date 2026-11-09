//! Generic equipment symbols retain their physical footprint and an explicit
//! fixed-paper mark. Elevations and sections use physical geometry.
use crate::{
    Model, ModelError,
    compiler::error,
    drawings::DrawingBody,
    plant_plans::{PlantPlan, project_sketch},
};
use multicad_core::{
    document::{Document, EntityData, EntityId},
    drawings::*,
    equipment, sketch, symbols,
};
use std::collections::{BTreeMap, BTreeSet};

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
    let definitions = symbols::definitions(document)
        .map_err(|e| error(None, e))?
        .into_iter()
        .map(|(id, definition)| {
            equipment::template_for(&definition.template, definition.root)
                .map(|template| (id, template))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()
        .map_err(|e| error(None, e))?;
    let mut settings = BTreeMap::new();
    for entity in document.entities() {
        if let EntityData::Extension { definition, .. } = &entity.data
            && definition.type_id == equipment::INSTANCE_TYPE
        {
            let value = equipment::InstanceSettings::from_definition(definition)
                .map_err(|e| error(Some(entity.id), e))?;
            settings.insert(value.symbol_instance, value);
        }
    }
    let parameters = multicad_core::parameters::evaluate(document).map_err(|e| error(None, e))?;
    let mut output = BTreeMap::new();
    for entity in document.entities() {
        let EntityData::Extension { definition, .. } = &entity.data else {
            continue;
        };
        if definition.type_id != symbols::INSTANCE_TYPE {
            continue;
        }
        let symbol =
            symbols::read_instance(document, entity.id).map_err(|e| error(Some(entity.id), e))?;
        if !selected.contains(&symbol.instance) {
            continue;
        }
        let Some(Some(template)) = definitions.get(&symbol.definition) else {
            continue;
        };
        if view
            .equipment_categories
            .as_ref()
            .is_some_and(|categories| !categories.contains(&template.kind.preset().category))
        {
            output.insert(
                symbol.instance,
                PlantPlan {
                    vectors: vec![],
                    labels: vec![],
                },
            );
            continue;
        }
        let pose = model
            .snapshot()
            .occurrences
            .get(&symbol.instance)
            .ok_or_else(|| {
                error(
                    Some(symbol.instance),
                    "Equipment has no current occurrence pose",
                )
            })?
            .pose;
        let footprint = multicad_solver::evaluate_document_sketch_with_parameters(
            document,
            revision,
            symbol.ids[&template.footprint],
            Some(&model.snapshot().references),
            &parameters,
        )
        .map_err(|e| error(Some(symbol.instance), e))?;
        if sketch::dot(pose.vector(footprint.frame.n), view.frame.normal()) < 1. - 1e-8 {
            continue;
        }
        let mut plan = PlantPlan {
            vectors: project_sketch(view, symbol.instance, pose, &footprint, 1.)?,
            labels: vec![],
        };
        let mut label_origin = pose.point(footprint.frame.point([0., 0.]));
        if let (Some(mark), Some(paper_mm)) = (template.mark, template.mark_paper_mm) {
            let mark = multicad_solver::evaluate_document_sketch_with_parameters(
                document,
                revision,
                symbol.ids[&mark],
                Some(&model.snapshot().references),
                &parameters,
            )
            .map_err(|e| error(Some(symbol.instance), e))?;
            let scale = paper_mm * view.scale.denominator as f64 / view.scale.numerator as f64;
            let mut mark_pose = pose;
            if matches!(
                template.mount,
                equipment::MountClass::WallCenter | equipment::MountClass::WallBottom
            ) {
                let depth = parameters
                    .value(
                        symbol.ids[&template.depth_parameter],
                        multicad_core::parameters::Unit::Millimeter,
                    )
                    .map_err(|e| error(Some(symbol.instance), e))?;
                let model_per_paper = view.scale.denominator as f64 / view.scale.numerator as f64;
                mark_pose.translation_mm =
                    pose.point([0., -depth - (paper_mm / 2. + 0.5) * model_per_paper, 0.]);
                let project = |world: [f64; 3]| {
                    let delta = std::array::from_fn(|i| world[i] - view.frame.origin_mm[i]);
                    [
                        sketch::dot(delta, view.frame.u),
                        sketch::dot(delta, view.frame.v),
                    ]
                };
                let a = project(label_origin);
                let b = project(mark_pose.translation_mm);
                plan.vectors.push(DrawingVector {
                    color_rgb: None,
                    source: symbol.instance,
                    material_part: symbol.instance,
                    role: VectorRole::Visible,
                    curve: VectorCurve::Line { a, b },
                    preview_mm: vec![a, b],
                    line_width_mm: 0.18,
                });
                label_origin = mark_pose.translation_mm;
            }
            plan.vectors.extend(project_sketch(
                view,
                symbol.instance,
                mark_pose,
                &mark,
                scale,
            )?);
        }
        if let Some(text) = settings.get(&entity.id).and_then(|s| s.label.clone()) {
            let world = label_origin;
            let delta = std::array::from_fn(|i| world[i] - view.frame.origin_mm[i]);
            let scale = view.scale.numerator as f64 / view.scale.denominator as f64;
            let right = plan
                .vectors
                .iter()
                .flat_map(|v| &v.preview_mm)
                .map(|p| p[0])
                .fold(sketch::dot(delta, view.frame.u), f64::max);
            plan.labels.push(SourceLabel {
                source: symbol.instance,
                at_mm: [right + 2. / scale, sketch::dot(delta, view.frame.v)],
                text,
            });
        }
        output.insert(symbol.instance, plan);
    }
    Ok(output)
}
