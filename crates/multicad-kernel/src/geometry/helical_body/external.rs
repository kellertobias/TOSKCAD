//! Cap-entry external threading retains the uncut tail and source charts.
use super::*;
use multicad_rust_kernel::threaded_shaft::Shaft;

pub(super) struct ExternalStock {
    pub(super) partial: Option<Shaft>,
    source: Option<BoreStock>,
    authored: Vec<TopologyElement>,
    requested_length: f64,
    stock_length: f64,
}
fn shaft_error(cause: ThreadError) -> GeometryError {
    error(
        OPERATION,
        if cause == ThreadError::Limit {
            GeometryErrorKind::Limit
        } else {
            GeometryErrorKind::Input
        },
        "Invalid external receiving thread closure",
    )
}
impl ExternalStock {
    fn partial(
        thread: Thread,
        radius: f64,
        height: f64,
        policy: GeometryPolicy,
    ) -> Result<Option<Shaft>, GeometryError> {
        if height > thread.spec().length_mm {
            policy.length(height - thread.spec().length_mm)?;
            if radius > thread.spec().major_radius_mm {
                policy.length(radius - thread.spec().major_radius_mm)?;
            }
            Shaft::new(thread, radius, height)
                .map(Some)
                .map_err(shaft_error)
        } else {
            Ok(None)
        }
    }
    pub(super) fn intervals(
        &self,
        geometry: Thread,
        frame: PlaneFrame,
    ) -> ReceivingThreadIntervals {
        ReceivingThreadIntervals {
            frame,
            requested_tool_length_mm: self.requested_length,
            physical_thread_length_mm: geometry.spec().length_mm,
            stock_length_mm: self.stock_length,
        }
    }
    pub(super) fn volume(&self, thread: Thread) -> f64 {
        self.partial
            .map_or_else(|| thread.volume_mm3(), |shaft| shaft.volume_mm3())
    }
    pub(super) fn contains(&self, thread: Thread, point: Point, tolerance: f64) -> bool {
        self.partial.map_or_else(
            || thread.contains(point, tolerance),
            |shaft| shaft.contains(point, tolerance),
        )
    }
    pub(super) fn support(&self, thread: Thread, direction: Point) -> f64 {
        self.partial.map_or_else(
            || thread.support(direction),
            |shaft| shaft.support(direction),
        )
    }
    pub(super) fn extend_properties(&self, thread: Thread, faces: &mut Vec<(Point, f64)>) {
        let Some(shaft) = self.partial else { return };
        let spec = thread.spec();
        let breaks = thread.profile_breaks();
        let rate = thread.helical_rate();
        let z = spec.length_mm;
        let periods = if shaft.tangent() { spec.starts } else { 1 };
        for period in 0..periods {
            let mut moments = [0.; 3];
            for k in if shaft.tangent() {
                period..period + 1
            } else {
                0..spec.starts
            } {
                let shift = k as f64 * spec.pitch_mm;
                for pair in breaks.windows(2) {
                    let part = integral(pair[0], pair[1], |phase| {
                        let r = thread.radius_at_phase(phase);
                        let (sin, cos) = ((z - shift - phase) / rate).sin_cos();
                        [
                            (shaft.radius_mm() - r) * (shaft.radius_mm() + r) / 2.,
                            (shaft.radius_mm() - r)
                                * (shaft.radius_mm().powi(2) + shaft.radius_mm() * r + r * r)
                                * cos
                                / 3.,
                            (shaft.radius_mm() - r)
                                * (shaft.radius_mm().powi(2) + shaft.radius_mm() * r + r * r)
                                * sin
                                / 3.,
                        ]
                    });
                    for (moment, term) in moments.iter_mut().zip(part) {
                        *moment += term / rate.abs();
                    }
                }
            }
            let property = (
                [moments[1] / moments[0], moments[2] / moments[0], z],
                moments[0],
            );
            if period == 0 {
                faces[1] = property;
            } else {
                faces.push(property);
            }
        }
        faces.push((
            [0., 0., z.midpoint(shaft.stock_length_mm())],
            TAU * shaft.radius_mm() * (shaft.stock_length_mm() - z),
        ));
        faces.push((
            [0., 0., shaft.stock_length_mm()],
            PI * shaft.radius_mm().powi(2),
        ));
    }
    pub(super) fn plane(&self, index: usize, frame: PlaneFrame) -> Option<ReferenceFrame> {
        let shaft = self.partial?;
        if (0..shaft.shoulder_count()).any(|k| index == shaft.shoulder_face(k) as usize) {
            Some(boundary::placed_frame(
                frame,
                [0., 0., shaft.thread().spec().length_mm],
                [1., 0., 0.],
                [0., -1., 0.],
            ))
        } else if index == shaft.far_cap_face() as usize {
            Some(boundary::placed_frame(
                frame,
                [0., 0., shaft.stock_length_mm()],
                [1., 0., 0.],
                [0., 1., 0.],
            ))
        } else {
            None
        }
    }
}
impl Solid {
    /// Form a straight modeled thread inside the cap-entry interval of one
    /// coaxial stock cylinder, retaining the actual untouched cylindrical tail.
    pub fn rust_cut_external_thread(
        &self,
        spec: HelicalThread,
        root_radius_mm: Option<f64>,
        feature: EntityId,
        frame: PlaneFrame,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        policy.validate()?;
        frame.checked(policy)?;
        // Validate the literal tool before considering a material prefix.
        let tool = checked_thread(spec, root_radius_mm, policy)?;
        frame.checked_metric_bounds(
            Bounds {
                min: [-spec.major_radius_mm, -spec.major_radius_mm, 0.],
                max: [spec.major_radius_mm, spec.major_radius_mm, spec.length_mm],
            },
            policy,
        )?;
        let SolidBody::AxialCylinder(cylinder, source_frame, sketch) = &self.body else {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "External receiving threads require one analytic stock cylinder",
            ));
        };
        let source_frame = *source_frame;
        let height = cylinder.height().as_f64();
        let radius = cylinder.circle().radius().as_f64();
        if radius < spec.major_radius_mm {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "External thread crest clipping by undersize stock is not certified",
            ));
        }
        if spec.length_mm < height && radius > spec.major_radius_mm {
            policy.length(radius - spec.major_radius_mm)?;
        }
        if spec.length_mm < height {
            policy.length(height - spec.length_mm)?;
        }
        if feature.uuid_bits() == cylinder.feature() {
            return Err(input(
                "The thread and receiving feature identities must be distinct",
            ));
        }
        let alignment = dot(frame.normal(), source_frame.normal());
        let reversed = alignment < 0.;
        let start = position(source_frame, [0., 0., if reversed { height } else { 0. }]);
        let distance = |a: Point, b: Point| {
            let [x, y, z] = std::array::from_fn(|i| a[i] - b[i]);
            x.hypot(y).hypot(z)
        };
        if self.policy != policy
            || policy.boolean_fuzzy_mm != 0.
            || (alignment.abs() - 1.).abs() > 1e-10
            || distance(start, frame.origin) > policy.linear_tolerance_mm
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "External receiving threads must enter a coaxial stock cap under the same exact policy",
            ));
        }
        let chart = stock_frame::Chart::new(source_frame);
        let [x, y, _] = chart.vector(frame.u);
        let norm = x.hypot(y);
        let [c, s] = [x / norm, y / norm];
        let sign = if reversed { -1. } else { 1. };
        let placed = PlaneFrame {
            origin: start,
            u: std::array::from_fn(|i| c * source_frame.u[i] + s * source_frame.v[i]),
            v: std::array::from_fn(|i| sign * (-s * source_frame.u[i] + c * source_frame.v[i])),
        };
        if distance(
            position(placed, [0., 0., spec.length_mm]),
            position(frame, [0., 0., spec.length_mm]),
        ) > policy.linear_tolerance_mm
        {
            return Err(error(
                OPERATION,
                GeometryErrorKind::Unsupported,
                "External thread cannot preserve its declared axial interval",
            ));
        }
        stock_frame::certify_tool_placement(frame, placed, &tool, policy)?;
        let prefix = if spec.length_mm > height {
            Some(Box::new(tool.axial_prefix(height).map_err(shaft_error)?))
        } else {
            None
        };
        let material = prefix.as_ref().map_or(tool, |prefix| *prefix.material());
        let stored = stock_frame::store(
            placed,
            source_frame,
            Bounds {
                min: [-radius, -radius, 0.],
                max: [radius, radius, height],
            },
            policy,
        )?;
        let snapshot = self.topology_snapshot(feature, 0)?;
        let anchors = stock_frame::store_anchors(placed, snapshot.clone(), policy)?;
        let authored = snapshot
            .elements
            .into_iter()
            .map(|mut element| {
                element.geometry = None;
                for anchor in &mut element.frames {
                    anchor.frame = anchors[&anchor.origin];
                }
                element
            })
            .collect();
        let stock = ExternalStock {
            partial: ExternalStock::partial(material, radius, height, policy)?,
            source: Some(BoreStock {
                source_feature: EntityId::from_uuid_bits(cylinder.feature()),
                radius_mm: radius,
                length_mm: height,
                frame_in_thread: stored,
                sketch: sketch.clone(),
                reversed,
                prefix,
            }),
            authored,
            requested_length: spec.length_mm,
            stock_length: height,
        };
        Self::publish_helical_geometry(material, feature, placed, policy, |_| {
            Ok(HelicalForm::External(Box::new(stock)))
        })
    }
    pub(in crate::geometry) fn import_threaded_shaft(
        spec: Spec,
        radius: f64,
        height: f64,
        frame: PlaneFrame,
        feature: EntityId,
        policy: GeometryPolicy,
    ) -> Result<Self, GeometryError> {
        let mut solid = Self::rust_helical_body(
            imported_request(spec),
            spec.root_radius_mm,
            feature,
            frame,
            policy,
            |thread| {
                let partial = ExternalStock::partial(thread, radius, height, policy)?
                    .ok_or_else(|| input("Imported external stock requires a positive tail"))?;
                Ok(HelicalForm::External(Box::new(ExternalStock {
                    partial: Some(partial),
                    source: None,
                    authored: vec![],
                    requested_length: spec.length_mm,
                    stock_length: height,
                })))
            },
        )?;
        let identity = super::super::step_import::feature_origin(
            feature,
            "threaded-shaft-material",
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
                    radius,
                    height,
                ]),
        );
        let SolidBody::Helical(body, _) = &mut solid.body else {
            unreachable!()
        };
        Rc::get_mut(body)
            .expect("unshared imported external thread")
            .import_identity = Some(identity);
        Ok(solid)
    }
}
impl ExternalStock {
    pub(super) fn extend_topology(
        &self,
        body: &HelicalBody,
        frame: PlaneFrame,
        _owner: EntityId,
        _revision: u64,
        count: usize,
        elements: &mut Vec<TopologyElement>,
    ) -> Result<(), GeometryError> {
        let face_key = |ordinal| TopologyKey {
            kind: TopologyKind::Face,
            ordinal,
        };
        let mut far_circle = None;
        let far_face = if let Some(shaft) = self.partial {
            let radius = shaft.radius_mm();
            let length = body.geometry.spec().length_mm;
            let at = |z| ReferenceFrame {
                origin_mm: position(frame, [0., 0., z]),
                u: frame.u,
                v: frame.v,
            };
            let tail = body.origin("connection.external/tail.wall".into());
            let far = body.origin("connection.external/tail.cap".into());
            let far_rim = body.origin("connection.external/tail.far-rim".into());
            let mut tail_boundary = BTreeSet::from([far_rim.clone()]);
            for period in 0..shaft.shoulder_count() {
                let role = if shaft.tangent() {
                    format!("connection.external/shoulder/start/{period}")
                } else {
                    "connection.external/shoulder".into()
                };
                let origin = body.origin(role);
                let outer = body.origin(format!("connection.external/shoulder/outer-rim/{period}"));
                let mut boundary = BTreeSet::from([outer.clone()]);
                if shaft.tangent() {
                    let start = period as usize * 4;
                    for rail in start..start + 3 {
                        boundary.insert(body.rim_origin(1, rail));
                    }
                    tail_boundary.insert(body.rim_origin(1, start + 3));
                } else {
                    for rail in 0..count {
                        boundary.insert(body.rim_origin(1, rail));
                    }
                }
                tail_boundary.insert(outer.clone());
                let plane = self
                    .plane(shaft.shoulder_face(period) as usize, frame)
                    .expect("shoulder plane");
                let element = TopologyElement {
                    key: face_key(shaft.shoulder_face(period)),
                    origins: BTreeSet::from([origin.clone()]),
                    boundary,
                    frames: vec![AnchoredFrame {
                        origin,
                        frame: plane,
                    }],
                    geometry: Some(AnalyticGeometry::Plane { frame: plane }),
                };
                if period == 0 {
                    elements[1] = element;
                } else {
                    elements.push(element);
                }
                let (start_rad, sweep_rad) = if shaft.tangent() {
                    let a = body.rails[period as usize * 4];
                    let b = body.rails[period as usize * 4 + 3];
                    let rate = body.geometry.helical_rate();
                    ((length - a) / rate, (a - b) / rate)
                } else {
                    (0., TAU)
                };
                elements.push(TopologyElement {
                    key: TopologyKey {
                        kind: TopologyKind::Edge,
                        ordinal: (3 * count) as u32 + period,
                    },
                    origins: BTreeSet::from([outer]),
                    boundary: BTreeSet::new(),
                    frames: vec![],
                    geometry: Some(AnalyticGeometry::CircularArc {
                        frame: at(length),
                        radius_mm: radius,
                        start_rad,
                        sweep_rad,
                    }),
                });
            }
            elements.push(TopologyElement {
                key: face_key(shaft.wall_face()),
                origins: BTreeSet::from([tail]),
                boundary: tail_boundary,
                frames: vec![],
                geometry: None,
            });
            let plane = at(shaft.stock_length_mm());
            elements.push(TopologyElement {
                key: face_key(shaft.far_cap_face()),
                origins: BTreeSet::from([far.clone()]),
                boundary: BTreeSet::from([far_rim.clone()]),
                frames: vec![AnchoredFrame {
                    origin: far,
                    frame: plane,
                }],
                geometry: Some(AnalyticGeometry::Plane { frame: plane }),
            });
            let key = TopologyKey {
                kind: TopologyKind::Edge,
                ordinal: (3 * count) as u32 + shaft.shoulder_count(),
            };
            elements.push(TopologyElement {
                key,
                origins: BTreeSet::from([far_rim]),
                boundary: BTreeSet::new(),
                frames: vec![],
                geometry: Some(AnalyticGeometry::CircularArc {
                    frame: plane,
                    radius_mm: radius,
                    start_rad: 0.,
                    sweep_rad: TAU,
                }),
            });
            far_circle = Some(key);
            shaft.far_cap_face()
        } else {
            1
        };
        if let Some(source) = &self.source {
            for original in &self.authored {
                let destination = match original.key {
                    TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: 0 | 1,
                    } => {
                        let cap = original.key.ordinal as usize ^ usize::from(source.reversed);
                        Some(face_key(if cap == 0 { 0 } else { far_face }))
                    }
                    TopologyKey {
                        kind: TopologyKind::Face,
                        ordinal: 2,
                    } => self
                        .partial
                        .filter(|shaft| !shaft.tangent())
                        .map(|shaft| face_key(shaft.wall_face())),
                    TopologyKey {
                        kind: TopologyKind::Edge,
                        ordinal: 0 | 1,
                    } => {
                        let cap = original.key.ordinal as usize ^ usize::from(source.reversed);
                        if cap == 1 { far_circle } else { None }
                    }
                    _ => {
                        return Err(error(
                            OPERATION,
                            GeometryErrorKind::InvalidResult,
                            "Unexpected external receiving source topology",
                        ));
                    }
                };
                if let Some(key) = destination {
                    let element = elements
                        .iter_mut()
                        .find(|element| element.key == key)
                        .expect("published source-corresponding element");
                    element.origins.extend(original.origins.iter().cloned());
                    // Whole stock circles are retained only at the untouched
                    // far cap. Trimmed cap faces keep their actual new wires.
                    if key.kind == TopologyKind::Face
                        && key.ordinal == far_face
                        && self.partial.is_some()
                    {
                        element.boundary.extend(original.boundary.iter().cloned());
                    }
                    element
                        .frames
                        .extend(original.frames.iter().map(|anchor| AnchoredFrame {
                            origin: anchor.origin.clone(),
                            frame: boundary::placed_frame(
                                frame,
                                anchor.frame.origin_mm,
                                anchor.frame.u,
                                anchor.frame.v,
                            ),
                        }));
                }
            }
        }
        Ok(())
    }
}
