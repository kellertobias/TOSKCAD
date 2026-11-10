//! Exact source curves for a verified imported normal sweep.
use super::{
    GeometryError, GeometryErrorKind, Mesh, PlaneFrame, Solid, SolidBody, normal_sweep_body,
};
use crate::{
    Bounds, Face,
    rust_prism_topology::{Builder, position},
};
use multicad_core::{
    document::EntityId,
    references::{AnalyticGeometry, Origin, ReferenceFrame, TopologySnapshot},
};
use multicad_rust_kernel::{
    display::{MeshError, Tessellation},
    normal_sweep::Edge as AnalyticEdge,
    spline_sweep::{Edge, Sweep},
    step_read::SplineEdge,
};

pub(crate) struct SplineSweepBody {
    pub(super) feature: EntityId,
    pub(super) sweep: Sweep,
    wall_areas: Vec<f64>,
    wall_origins: Vec<Origin>,
}

impl Solid {
    pub(crate) fn spline_sweep_body(&self) -> Option<(&SplineSweepBody, PlaneFrame)> {
        if let SolidBody::SplineSweep(body, frame) = &self.body {
            Some((body, *frame))
        } else {
            None
        }
    }
}

impl SplineSweepBody {
    pub(super) fn new(
        feature: EntityId,
        sweep: Sweep,
        frame: PlaneFrame,
        tolerance: f64,
    ) -> Result<Self, GeometryError> {
        let wall_areas = sweep
            .outline()
            .iter()
            .map(|edge| {
                let [_, upper] = edge.length_bounds_mm(tolerance).map_err(|cause| {
                    super::error(
                        "STEP import",
                        match cause {
                            multicad_rust_kernel::spline_sweep::Error::Invalid => {
                                GeometryErrorKind::InvalidResult
                            }
                            multicad_rust_kernel::spline_sweep::Error::Limit => {
                                GeometryErrorKind::Limit
                            }
                        },
                        "Cannot bound the imported spline wall area",
                    )
                })?;
                Ok(upper * sweep.height_mm())
            })
            .chain(
                sweep
                    .hole()
                    .map(|hole| Ok(hole.length_mm() * sweep.height_mm())),
            )
            .collect::<Result<_, GeometryError>>()?;
        let wall_origins = sweep
            .outline()
            .iter()
            .map(|edge| {
                use super::step_import::{feature_origin, spline_control_key};
                match edge {
                    Edge::Cubic(cubic) => {
                        let poles: Vec<_> = cubic
                            .poles()
                            .iter()
                            .map(|&point| frame.point(point))
                            .collect();
                        let key = spline_control_key(&poles, cubic.knots());
                        feature_origin(feature, "cubic-wall", key.into_iter().map(f64::from_bits))
                    }
                    Edge::Analytic(edge) => {
                        let [a, middle, b] = [0., 0.5, 1.].map(|t| frame.point(edge.point(t)));
                        let ends = if a < b { [a, b] } else { [b, a] };
                        feature_origin(
                            feature,
                            "spline-sweep-wall",
                            ends.into_iter().chain([middle]).flatten(),
                        )
                    }
                }
            })
            .chain(sweep.hole().map(|hole| {
                super::step_import::feature_origin(
                    feature,
                    "spline-bore",
                    frame
                        .point(hole.centroid())
                        .into_iter()
                        .chain(hole.radius_mm()),
                )
            }))
            .collect();
        Ok(Self {
            feature,
            sweep,
            wall_areas,
            wall_origins,
        })
    }

    fn cap(&self, frame: PlaneFrame, top: bool) -> ReferenceFrame {
        let height = if top { self.sweep.height_mm() } else { 0. };
        ReferenceFrame {
            origin_mm: position(frame, [0., 0., height]),
            u: frame.u,
            v: frame.v,
        }
    }

    pub(crate) fn axial_drawing_rims(
        &self,
        frame: PlaneFrame,
        top: bool,
    ) -> (
        PlaneFrame,
        &[Edge],
        impl Iterator<Item = AnalyticGeometry> + '_,
    ) {
        let cap = self.cap(frame, top);
        let positioned = PlaneFrame {
            origin: cap.origin_mm,
            ..frame
        };
        let rims = self
            .sweep
            .outline()
            .iter()
            .filter_map(|edge| match edge {
                Edge::Analytic(edge) => Some(*edge),
                Edge::Cubic(_) => None,
            })
            .chain(self.sweep.hole())
            .map(move |edge| normal_sweep_body::rim_in_frame(edge, cap));
        (positioned, self.sweep.outline(), rims)
    }

    fn side_plane(frame: PlaneFrame, edge: &Edge) -> Option<ReferenceFrame> {
        let Edge::Analytic(AnalyticEdge::Line { a, b }) = edge else {
            return None;
        };
        let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
        let length = dx.hypot(dy);
        Some(ReferenceFrame {
            origin_mm: frame.point(*a),
            u: std::array::from_fn(|i| (dx * frame.u[i] + dy * frame.v[i]) / length),
            v: frame.normal(),
        })
    }

    pub(super) fn bounds(&self, frame: PlaneFrame) -> Bounds {
        let normal = frame.normal();
        let support = |i: usize, sign: f64| {
            self.sweep
                .support([sign * frame.u[i], sign * frame.v[i], sign * normal[i]])
        };
        Bounds {
            min: std::array::from_fn(|i| frame.origin[i] - support(i, -1.)),
            max: std::array::from_fn(|i| frame.origin[i] + support(i, 1.)),
        }
    }

    pub(super) fn faces(&self, frame: PlaneFrame) -> Vec<Face> {
        let [cx, cy, middle] = self.sweep.centroid_mm();
        let normal = frame.normal();
        let caps = [false, true].into_iter().map(|top| {
            let cap = self.cap(frame, top);
            Face {
                index: u32::from(top),
                plane: Some((cap.origin_mm, normal.map(|v| if top { v } else { -v }))),
                center_mm: position(frame, [cx, cy, if top { 2. * middle } else { 0. }]),
                area_mm2: self.sweep.area_mm2(),
            }
        });
        let wall = |index: usize, [x, y]: [f64; 2], support: Option<ReferenceFrame>| Face {
            index: index as u32 + 2,
            plane: support.map(|plane| (plane.origin_mm, plane.normal())),
            center_mm: position(frame, [x, y, middle]),
            area_mm2: self.wall_areas[index],
        };
        let outline = self.sweep.outline();
        caps.chain(outline.iter().enumerate().map(|(i, edge)| {
            let center = match edge {
                Edge::Analytic(edge) => edge.centroid(),
                Edge::Cubic(_) => edge.point(0.5),
            };
            wall(i, center, Self::side_plane(frame, edge))
        }))
        .chain(
            self.sweep
                .hole()
                .map(|hole| wall(outline.len(), hole.centroid(), None)),
        )
        .collect()
    }

    pub(super) fn mesh(
        &self,
        frame: PlaneFrame,
        options: Tessellation,
    ) -> Result<Mesh, GeometryError> {
        crate::rust_display::placed_mesh(self.sweep.mesh(options), frame, false)
    }

    pub(crate) fn topology(
        &self,
        frame: PlaneFrame,
        owner: EntityId,
        revision: u64,
    ) -> TopologySnapshot {
        let outline = self.sweep.outline();
        let count = outline.len();
        let wall_count = self.wall_areas.len();
        let origins = ["cap.start".into(), "cap.end".into()]
            .into_iter()
            .map(|role| {
                vec![Origin {
                    source: self.feature,
                    role,
                }]
            })
            .chain(self.wall_origins.iter().cloned().map(|origin| vec![origin]))
            .collect();
        let mut build = Builder::new(origins);
        for top in [false, true] {
            let cap_index = usize::from(top);
            let cap = self.cap(frame, top);
            let outward = if top {
                cap
            } else {
                ReferenceFrame {
                    v: cap.v.map(|v| -v),
                    ..cap
                }
            };
            build.face(cap_index, Some(outward), Some(cap), 2..wall_count + 2);
            for (index, edge) in outline.iter().enumerate() {
                let incident = [cap_index, index + 2];
                match edge {
                    Edge::Analytic(edge) => {
                        build.non_face(normal_sweep_body::rim_in_frame(*edge, cap), &incident)
                    }
                    Edge::Cubic(_) => build.non_analytic_edge(&incident),
                }
                let [x, y] = edge.point(0.);
                let previous = (index + count - 1) % count;
                build.non_face(
                    AnalyticGeometry::Point {
                        position_mm: position(
                            frame,
                            [x, y, if top { self.sweep.height_mm() } else { 0. }],
                        ),
                    },
                    &[cap_index, previous + 2, index + 2],
                );
            }
            if let Some(hole) = self.sweep.hole() {
                build.non_face(
                    normal_sweep_body::rim_in_frame(hole, cap),
                    &[cap_index, count + 2],
                );
            }
        }
        for (index, edge) in outline.iter().enumerate() {
            let wall = index + 2;
            let previous = (index + count - 1) % count + 2;
            let next = (index + 1) % count + 2;
            let plane = Self::side_plane(frame, edge);
            build.face(wall, plane, plane, [0, 1, previous, next]);
            let [x, y] = edge.point(0.);
            build.non_face(
                AnalyticGeometry::Line {
                    a_mm: position(frame, [x, y, 0.]),
                    b_mm: position(frame, [x, y, self.sweep.height_mm()]),
                },
                &[previous, wall],
            );
        }
        if self.sweep.hole().is_some() {
            build.face(count + 2, None, None, [0, 1]);
        }
        TopologySnapshot {
            owner,
            revision,
            elements: build.into_elements(),
        }
    }

    pub(super) fn matches_splines(
        &self,
        frame: PlaneFrame,
        readback: &[SplineEdge],
        tolerance: f64,
    ) -> bool {
        let project = |point: [f64; 3]| super::dot(point, frame.u);
        let mut starts: Vec<_> = readback
            .iter()
            .enumerate()
            .flat_map(|(i, curve)| {
                let poles = curve.poles_mm();
                [
                    (project(poles[0]), i, false),
                    (project(poles[poles.len() - 1]), i, true),
                ]
            })
            .collect();
        starts.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut used = vec![false; readback.len()];
        let mut count = 0;
        for cubic in self.sweep.outline().iter().filter_map(|edge| match edge {
            Edge::Cubic(cubic) => Some(cubic),
            Edge::Analytic(_) => None,
        }) {
            for z in [0., self.sweep.height_mm()] {
                let place = |[x, y]: [f64; 2]| position(frame, [x, y, z]);
                let start = project(place(cubic.point(0.)));
                let index = starts.partition_point(|entry| entry.0 < start - tolerance * 2.);
                let Some((_, found, _)) = starts[index..]
                    .iter()
                    .take_while(|entry| entry.0 <= start + tolerance * 2.)
                    .find(|&&(_, i, reversed)| {
                        let mut actual = readback[i].bezier_spans();
                        let mut expected = cubic.bezier_spans();
                        !used[i]
                            && actual.len() == expected.len()
                            && expected.all(|span| {
                                let actual = if reversed {
                                    let mut span =
                                        actual.next_back().expect("matching span counts");
                                    span.reverse();
                                    span
                                } else {
                                    actual.next().expect("matching span counts")
                                };
                                // A cubic remains inside its Bezier control hull.
                                // Comparing every matched control bounds the whole trim.
                                span.map(place).into_iter().zip(actual).all(|(a, b)| {
                                    a.into_iter()
                                        .zip(b)
                                        .all(|(a, b)| (a - b).abs() <= tolerance)
                                })
                            })
                    })
                else {
                    return false;
                };
                used[*found] = true;
                count += 1;
            }
        }
        count == readback.len()
    }

    pub(super) fn sample_rim(
        &self,
        frame: PlaneFrame,
        ordinal: u32,
        options: Tessellation,
    ) -> Option<Result<Vec<[f64; 3]>, MeshError>> {
        let stride = self.wall_areas.len();
        let index = ordinal as usize;
        let edge = self.sweep.outline().get(index % stride)?;
        if index >= 2 * stride || !matches!(edge, Edge::Cubic(_)) {
            return None;
        }
        let height = if index >= stride {
            self.sweep.height_mm()
        } else {
            0.
        };
        Some(edge.sample(options).map(|points| {
            points
                .into_iter()
                .map(|[x, y]| position(frame, [x, y, height]))
                .collect()
        }))
    }
}
