use crate::{
    camera::Camera,
    egui,
    viewport::{Scene, SectionPlane},
};
use multicad_core::{
    inspection::{GeometryReference, MeasurementQuery, MeasurementValues, ScalarMetric},
    references::{AnalyticGeometry, TopologyKey, TopologyKind},
    sketch::dot,
};

#[derive(Default)]
pub(super) struct AngleMarker {
    pub rays: [Option<[egui::Pos2; 2]>; 2],
    pub arc: Vec<[egui::Pos2; 2]>,
}

pub(super) fn marker(
    scene: &Scene,
    query: &MeasurementQuery,
    values: &MeasurementValues,
    camera: Camera,
    rect: egui::Rect,
    section: Option<SectionPlane>,
) -> Option<AngleMarker> {
    let ScalarMetric::Radians { value } = values.angle else {
        return None;
    };
    if !scene.is_full_model()
        || matches!(
            values.parallel_plane_separation,
            ScalarMetric::Millimeters { .. }
        )
    {
        return None;
    }
    let witness = values.witnesses.first()?;
    let midpoint = std::array::from_fn(|i| witness.a_mm[i] * 0.5 + witness.b_mm[i] * 0.5);
    project(
        midpoint,
        [
            normal(scene, &query.a, values.key_a)?,
            normal(scene, &query.b, values.key_b)?,
        ],
        value,
        camera,
        rect,
        section,
    )
}

fn normal(scene: &Scene, reference: &GeometryReference, key: TopologyKey) -> Option<[f64; 3]> {
    if key.kind != TopologyKind::Face || reference.intent.kind != TopologyKind::Face {
        return None;
    }
    match scene.resolved_geometry(reference.source, key, &reference.intent)? {
        AnalyticGeometry::Plane { frame } => Some(frame.normal()),
        _ => None,
    }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(vector: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(vector, vector).sqrt();
    (length.is_finite() && length > 1e-10).then(|| vector.map(|value| value / length))
}

fn project(
    origin: [f64; 3],
    [normal_a, normal_b]: [[f64; 3]; 2],
    radians: f64,
    camera: Camera,
    rect: egui::Rect,
    section: Option<SectionPlane>,
) -> Option<AngleMarker> {
    if !rect.is_finite()
        || rect.width() <= 0.
        || rect.height() <= 0.
        || !radians.is_finite()
        || radians <= 0.
        || radians > std::f64::consts::FRAC_PI_2
        || origin.iter().any(|value| !value.is_finite())
        || section.is_some_and(|plane| !plane.contains(origin))
        || !camera.scale.is_finite()
        || camera.scale <= 0.
    {
        return None;
    }

    let axis = unit(cross(normal_a, normal_b))?;
    let a = unit(cross(axis, normal_a))?;
    let mut b = unit(cross(axis, normal_b))?;
    let cosine = dot(a, b);
    // The native angle is unoriented and acute. Refuse a guide if the exact
    // current plane directions do not agree with that measurement receipt.
    if (cosine.abs() - radians.cos()).abs() > 1e-9 {
        return None;
    }
    if cosine < -1e-10 {
        b = b.map(|value| -value);
    }
    let across = unit(std::array::from_fn(|i| b[i] - dot(a, b) * a[i]))?;

    let [x, y, depth] = camera.project(origin.map(|value| value as f32), rect);
    let center = egui::pos2(x, y);
    let view = camera.ray(center, rect)?;
    // An edge-on cross-section collapses to a line. A screen-space circular
    // arc would incorrectly imply that its opening is visible.
    if camera.perspective && depth <= 0.1 {
        return None;
    }
    if dot(view.direction(), axis).abs() < 0.04 {
        return Some(AngleMarker::default());
    }

    // The virtual guides scale around the witness, accounting for perspective
    // depth there. Keep the full guide ahead of the perspective near plane.
    let scale = f64::from(camera.scale);
    let focal_distance = f64::from(rect.height().max(1.)) / scale * 2.;
    let mut pixels_per_mm = scale;
    if camera.perspective {
        pixels_per_mm *= focal_distance / f64::from(depth);
    }
    let target_pixels = f64::from((rect.width().min(rect.height()) * 0.12).clamp(20., 62.));
    let mut length = target_pixels / pixels_per_mm;
    if camera.perspective {
        length = length.min(f64::from(depth - 0.1) * 0.2);
    }
    if !length.is_finite() || length <= 0. {
        return None;
    }
    let point = |direction: [f64; 3], radius: f64| {
        std::array::from_fn(|i| origin[i] + direction[i] * radius)
    };
    let sweep = |angle: f64| {
        let (sine, cosine) = angle.sin_cos();
        std::array::from_fn(|i| a[i] * cosine + across[i] * sine)
    };
    let steps = (radians * 24. / std::f64::consts::FRAC_PI_2).ceil() as i32;
    let arc_point =
        |index, radius| point(sweep(radians * f64::from(index) / f64::from(steps)), radius);
    // Clamp the screen span again after projection. Off-axis perspective
    // can magnify a direction even at the same witness depth.
    let projected_extent = [point(a, length), point(b, length)]
        .into_iter()
        .chain((0..=steps).map(|index| arc_point(index, length * 0.66)))
        .map(|point| {
            let [x, y, _] = camera.project(point.map(|value| value as f32), rect);
            f64::from(center.distance(egui::pos2(x, y)))
        })
        .fold(0., f64::max);
    if !projected_extent.is_finite() {
        return None;
    }
    if projected_extent < 2. {
        return Some(AngleMarker::default());
    }
    length *= (target_pixels / projected_extent).min(1.);

    let segment = |a, b| {
        let [a, b] = section.map_or(Some([a, b]), |plane| plane.clip_segment(a, b))?;
        let segment = camera.segment(a, b, rect)?;
        (segment.iter().all(|point| point.is_finite()) && segment[0].distance(segment[1]) > 0.25)
            .then_some(segment)
    };
    let rays = [a, b].map(|direction| {
        segment(origin, point(direction, length)).filter(|[a, b]| a.distance(*b) > 1.)
    });
    let radius = length * 0.66;
    let arc = (0..steps)
        .filter_map(|index| segment(arc_point(index, radius), arc_point(index + 1, radius)))
        .collect();
    Some(AngleMarker { rays, arc })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NORMALS: [[f64; 3]; 2] = [[1., 0., 0.], [0., 0., 1.]];

    fn rect() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.))
    }

    fn camera(perspective: bool) -> Camera {
        Camera {
            yaw: 0.,
            pitch: 0.,
            target: [0.; 3],
            perspective,
            ..Default::default()
        }
    }

    #[test]
    fn marker_uses_the_exact_occurrence_key_and_world_frame() {
        use crate::viewport::{Geometry, Instance, SceneStamp, Transform};
        use multicad_core::{
            document::EntityId,
            inspection::MeasurementWitness,
            model::{ModelEvaluationInfo, RigidPose},
            project_file::ContentHash,
            references::{GeometryIntent, Origin, ReferenceFrame, TopologyElement},
        };
        use std::sync::Arc;

        let [document, producer, a, b] = std::array::from_fn(|_| EntityId::new());
        let key = TopologyKey {
            kind: TopologyKind::Face,
            ordinal: 0,
        };
        let plane = ReferenceFrame {
            origin_mm: [0.; 3],
            u: [0., 1., 0.],
            v: [0., 0., 1.],
        };
        let geometry = Geometry::new(
            producer,
            vec![[0.; 3], plane.u, plane.v],
            vec![0, 1, 2],
            vec![0],
            vec![TopologyElement {
                key,
                origins: [Origin {
                    source: producer,
                    role: "side".into(),
                }]
                .into(),
                boundary: Default::default(),
                frames: vec![],
                geometry: Some(AnalyticGeometry::Plane { frame: plane }),
            }],
        )
        .unwrap();
        let rotated = RigidPose {
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
            translation_mm: [0., 0., 4.],
        };
        let instance = |id, transform| Instance {
            id,
            geometry: producer,
            transform,
            color: [255; 4],
        };
        let scene = Scene::new(
            SceneStamp {
                document,
                revision: 9,
                generation: 1,
            },
            vec![Arc::new(geometry)],
            vec![
                instance(a, Transform::default()),
                instance(b, Transform::from_rigid_pose(rotated).unwrap()),
            ],
        )
        .unwrap()
        .with_model_evaluation(Arc::new(ModelEvaluationInfo {
            source_hash: ContentHash::of(b"current model"),
            consumed_bodies: Default::default(),
            cut_targets: vec![],
            screw_connection_checks: vec![],
        }))
        .unwrap();
        let reference = |source| GeometryReference {
            source,
            intent: GeometryIntent::capture(&scene.topology_snapshot(source).unwrap(), key)
                .unwrap(),
        };
        let mut query = MeasurementQuery {
            a: reference(a),
            b: reference(b),
        };
        let mut values = MeasurementValues {
            minimum_mm: 4.,
            witnesses: vec![MeasurementWitness {
                a_mm: [0.; 3],
                b_mm: rotated.translation_mm,
            }],
            witnesses_complete: true,
            total_solutions: Some(1),
            inner_solution: false,
            axis_components_mm: None,
            axis_warning: None,
            angle: ScalarMetric::Radians {
                value: std::f64::consts::FRAC_PI_2,
            },
            parallel_plane_separation: ScalarMetric::Unsupported {
                reason: "not parallel".into(),
            },
            tolerance_mm: 1e-7,
            boolean_fuzzy_mm: 0.,
            key_a: key,
            key_b: key,
        };
        let camera = Camera {
            target: [0., 0., 2.],
            ..camera(false)
        };
        let guide = marker(&scene, &query, &values, camera, rect(), None).unwrap();
        assert!(!guide.arc.is_empty());
        for ray in guide.rays {
            assert_eq!(ray.unwrap()[0], rect().center());
        }
        let section = SectionPlane::new([0., 0., 1.], 3.).unwrap();
        assert!(marker(&scene, &query, &values, camera, rect(), Some(section)).is_none());

        values.key_b.ordinal = 1;
        assert!(marker(&scene, &query, &values, camera, rect(), None).is_none());
        values.key_b = key;
        query.b.intent.origins = [Origin {
            source: EntityId::new(),
            role: "side".into(),
        }]
        .into();
        assert!(marker(&scene, &query, &values, camera, rect(), None).is_none());
    }

    #[test]
    fn square_on_world_guides_use_the_exact_acute_angle_in_both_projections() {
        let angle = std::f64::consts::FRAC_PI_3;
        let normals = [[1., 0., 0.], [angle.cos(), 0., angle.sin()]];
        for perspective in [false, true] {
            for scale in [0.15, 2.4, 35.] {
                let marker = project(
                    [0., 0., 0.],
                    normals,
                    angle,
                    Camera {
                        scale,
                        ..camera(perspective)
                    },
                    rect(),
                    None,
                )
                .unwrap();
                let [a, b] = marker.rays.map(|ray| {
                    let [start, end] = ray.unwrap();
                    assert_eq!(start, rect().center());
                    assert!((50. ..=65.).contains(&start.distance(end)));
                    (end - start).normalized()
                });
                assert!((f64::from(a.dot(b).acos()) - angle).abs() < 1e-6);
                let arc_start = marker.arc[0][0];
                let arc_end = marker.arc.last().unwrap()[1];
                assert!((arc_start - rect().center()).normalized().dot(a) > 0.9999);
                assert!((arc_end - rect().center()).normalized().dot(b) > 0.9999);
            }
        }
    }

    #[test]
    fn a_small_nonparallel_angle_keeps_its_world_arc() {
        let angle = 3_f64.to_radians();
        let normals = [[1., 0., 0.], [angle.cos(), 0., angle.sin()]];
        let marker = project([0.; 3], normals, angle, camera(false), rect(), None).unwrap();
        assert!(!marker.arc.is_empty());
        assert!(marker.rays.iter().all(Option::is_some));
    }

    #[test]
    fn section_clips_the_guides_and_edge_on_view_keeps_only_the_exact_angle() {
        let angle = std::f64::consts::FRAC_PI_2;
        let section = SectionPlane::new([1., 0., 0.], -6.).unwrap();
        let clipped = project(
            [0.; 3],
            NORMALS,
            angle,
            camera(false),
            rect(),
            Some(section),
        )
        .unwrap();
        assert!(clipped.arc.len() < 24);
        let left_edge = rect().center().x - 6. * camera(false).scale;
        for p in clipped
            .rays
            .into_iter()
            .flatten()
            .chain(clipped.arc)
            .flatten()
        {
            assert!(p.x >= left_edge - 0.001);
        }
        let edge_on = Camera {
            yaw: std::f32::consts::FRAC_PI_2,
            ..camera(false)
        };
        let collapsed = project([0.; 3], NORMALS, angle, edge_on, rect(), None).unwrap();
        assert!(collapsed.arc.is_empty() && collapsed.rays.iter().all(Option::is_none));
        assert!(project([0.; 3], [NORMALS[0]; 2], 0., camera(false), rect(), None).is_none());
        assert!(project([0.; 3], NORMALS, 0.5, camera(false), rect(), None).is_none());
    }
}
