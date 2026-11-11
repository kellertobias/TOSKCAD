//! Terminal neck material retains its original tool and actual shoulder station.
use super::*;
use multicad_rust_kernel::thread_end_relief::Relief;

fn relief_error(cause: ThreadError) -> GeometryError {
    error(
        OPERATION,
        if cause == ThreadError::Limit {
            GeometryErrorKind::Limit
        } else {
            GeometryErrorKind::Input
        },
        "Invalid terminal thread relief material",
    )
}
fn checked_material(relief: Relief, policy: GeometryPolicy) -> Result<Relief, GeometryError> {
    for length in [relief.material_length_mm(), relief.neck_radius_mm()] {
        policy.length(length)?;
    }
    if let Some(intent) = relief.intent() {
        // Canonical dimensions are the authored scalars, not a second rounded
        // subtraction of the physical shoulder from the far station.
        policy.length(intent.width_mm)?;
        policy.length(intent.radial_depth_mm)?;
    } else {
        let physical_width = relief.full_length_mm() - relief.material_length_mm();
        if policy.length(physical_width).is_err() {
            // Exchange has no authored width. An exact minimum-width preimage
            // still proves this same material could be an admitted source;
            // neither station nor the policy threshold is moved or fuzzed.
            let minimum = policy.minimum_feature_mm;
            let source = relief.source_thread();
            if minimum < 0.001
                || minimum + source.spec().pitch_mm > relief.full_length_mm()
                || relief.full_length_mm() - minimum != relief.material_length_mm()
            {
                policy.length(physical_width)?;
            }
        }
    }
    Ok(relief)
}
impl Solid {
    /// Exact terminal-relief material and its retained placement. Consumers must
    /// use the complete descriptor, including the neck and actual far cap.
    pub fn rust_thread_end_relief_data(&self) -> Option<(Relief, PlaneFrame)> {
        let (body, frame) = self.helical_body()?;
        let HelicalForm::EndRelief(relief) = &body.form else {
            return None;
        };
        Some((*relief, frame))
    }
    /// Preserve an entire authored thread tool while replacing its terminal
    /// interval with an exact cylindrical neck. Entry remains a flat cut.
    #[allow(clippy::too_many_arguments)]
    pub fn rust_helical_thread_end_relief(
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        width_mm: f64,
        radial_depth_mm: f64,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        let tool = checked_thread(spec, root_radius_mm, policy)?;
        frame.checked_metric_bounds(
            Bounds {
                min: [-spec.major_radius_mm, -spec.major_radius_mm, 0.],
                max: [spec.major_radius_mm, spec.major_radius_mm, spec.length_mm],
            },
            policy,
        )?;
        policy.length(width_mm)?;
        policy.length(radial_depth_mm)?;
        let relief = checked_material(
            Relief::new(tool, width_mm, radial_depth_mm).map_err(relief_error)?,
            policy,
        )?;
        Self::publish_helical_geometry(*relief.material_thread(), feature, frame, policy, |_| {
            Ok(HelicalForm::EndRelief(relief))
        })
    }
    pub(in crate::geometry) fn import_thread_end_relief(
        spec: Spec,
        material_length: f64,
        neck_radius: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let tool = checked_thread(imported_request(spec), spec.root_radius_mm, policy)?;
        let relief = checked_material(
            Relief::from_material(tool, material_length, neck_radius).map_err(relief_error)?,
            policy,
        )?;
        let mut solid = Self::publish_helical_geometry(
            *relief.material_thread(),
            feature,
            frame,
            policy,
            |_| Ok(HelicalForm::EndRelief(relief)),
        )?;
        // Imported topology has finite full-material identity, never volatile
        // end ordinals alone. Changed pitch, phase, hand or neck requires repair.
        let identity = super::super::step_import::feature_origin(
            feature,
            "terminal-thread-relief-material",
            frame
                .origin
                .into_iter()
                .chain(frame.u)
                .chain(frame.v)
                .chain([
                    spec.major_radius_mm,
                    spec.pitch_mm,
                    spec.length_mm,
                    spec.depth_mm,
                    spec.crest_width_mm,
                    spec.lower_flank_deg,
                    spec.upper_flank_deg,
                    spec.starts as f64,
                    f64::from(spec.left_hand),
                    spec.root_radius_mm.unwrap_or(0.),
                    material_length,
                    neck_radius,
                ]),
        );
        let SolidBody::Helical(body, _) = &mut solid.body else {
            unreachable!()
        };
        Rc::get_mut(body)
            .expect("unshared imported terminal relief")
            .import_identity = Some(identity);
        Ok(solid)
    }
}
pub(super) fn extend_properties(relief: &Relief, faces: &mut Vec<(Point, f64)>) {
    let radius = relief.neck_radius_mm();
    let disk = PI * radius.powi(2);
    let (center, area) = &mut faces[1];
    let shoulder = *area - disk;
    center[0] *= *area / shoulder;
    center[1] *= *area / shoulder;
    *area = shoulder;
    faces.push((
        [
            0.,
            0.,
            relief
                .material_length_mm()
                .midpoint(relief.full_length_mm()),
        ],
        TAU * radius * (relief.full_length_mm() - relief.material_length_mm()),
    ));
    faces.push(([0., 0., relief.full_length_mm()], disk));
}
pub(super) fn plane(relief: &Relief, index: usize, frame: PlaneFrame) -> Option<ReferenceFrame> {
    (index == relief.far_cap_face() as usize).then(|| {
        boundary::placed_frame(
            frame,
            [0., 0., relief.full_length_mm()],
            [1., 0., 0.],
            [0., 1., 0.],
        )
    })
}
pub(super) fn extend_topology(
    body: &HelicalBody,
    relief: &Relief,
    frame: PlaneFrame,
    elements: &mut Vec<TopologyElement>,
) {
    let count = body.rails.len();
    let rim = body.origin("thread.end-relief/neck.entry-rim".into());
    let far_rim = body.origin("cap.end/neck-rim".into());
    let wall = body.origin("thread.end-relief/neck.wall".into());
    let far = body.origin("cap.end".into());
    elements[1].boundary.insert(rim.clone());
    elements.push(TopologyElement {
        key: TopologyKey {
            kind: TopologyKind::Face,
            ordinal: relief.wall_face(),
        },
        origins: BTreeSet::from([wall]),
        boundary: BTreeSet::from([rim.clone(), far_rim.clone()]),
        frames: vec![],
        geometry: None,
    });
    let cap = plane(relief, relief.far_cap_face() as usize, frame).expect("terminal cap");
    elements.push(TopologyElement {
        key: TopologyKey {
            kind: TopologyKind::Face,
            ordinal: relief.far_cap_face(),
        },
        origins: BTreeSet::from([far.clone()]),
        boundary: BTreeSet::from([far_rim.clone()]),
        frames: vec![AnchoredFrame {
            origin: far,
            frame: cap,
        }],
        geometry: Some(AnalyticGeometry::Plane { frame: cap }),
    });
    for (index, (origin, z)) in [
        (rim, relief.material_length_mm()),
        (far_rim, relief.full_length_mm()),
    ]
    .into_iter()
    .enumerate()
    {
        elements.push(TopologyElement {
            key: TopologyKey {
                kind: TopologyKind::Edge,
                ordinal: (3 * count + index) as u32,
            },
            origins: BTreeSet::from([origin]),
            boundary: BTreeSet::new(),
            frames: vec![],
            geometry: Some(AnalyticGeometry::CircularArc {
                frame: boundary::placed_frame(frame, [0., 0., z], [1., 0., 0.], [0., 1., 0.]),
                radius_mm: relief.neck_radius_mm(),
                start_rad: 0.,
                sweep_rad: TAU,
            }),
        });
    }
}
