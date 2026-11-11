//! Bounded, exact native relief on selected finite planar faces.
use crate::{
    GeometryBackend, ModelError,
    compiler::{checkpoint, error},
    face_profile::FaceProfileInputs,
};
use multicad_core::{
    document::EntityId,
    model::{
        self, MAX_SURFACE_STRUCTURE_MOTIFS, ResolvedSurfaceStructure, SurfaceStructureDirection,
        SurfaceStructurePattern,
    },
    parameters::EvaluatedParameters,
    project_file::ContentHash,
    references::Origin,
};
use multicad_kernel::{
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryErrorKind, GeometryPolicy,
    NormalReliefMotif, PlaneFrame, Solid,
};
use std::f64::consts::TAU;

const MAX_WAVE_SEGMENTS: usize = 24;
const MIN_RECESS_FLOOR_MM: f64 = 0.1;

struct Motif {
    index: usize,
    boundary: Vec<CurvedEdge2d>,
    height_mm: f64,
}

fn polygon(points: &[[f64; 2]]) -> Vec<CurvedEdge2d> {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(&a, &b)| CurvedEdge2d::Line { a, b })
        .collect()
}

fn rectangle([x, y]: [f64; 2], [w, h]: [f64; 2]) -> Vec<CurvedEdge2d> {
    let [u, v] = [w / 2., h / 2.];
    polygon(&[
        [x - u, y - v],
        [x + u, y - v],
        [x + u, y + v],
        [x - u, y + v],
    ])
}

fn center(index: usize, count: usize, pitch: f64) -> f64 {
    (index as f64 + 0.5 - count as f64 / 2.) * pitch
}

fn count(span: f64, pitch: f64) -> Result<usize, &'static str> {
    let cells = (span / pitch).floor();
    if !cells.is_finite() || cells > MAX_SURFACE_STRUCTURE_MOTIFS as f64 {
        return Err("Increase spacing or reduce the relief window to fit the motif budget");
    }
    if cells < 1. {
        return Err("The relief window needs space for at least one full pattern pitch");
    }
    Ok(cells as usize)
}

fn motifs(settings: &ResolvedSurfaceStructure, limit: usize) -> Result<Vec<Motif>, &'static str> {
    use SurfaceStructurePattern::{HeightMap, Ribs, Stipple, Waves};
    let [w, h] = settings.size_mm;
    let pitch = settings.pitch_mm;
    let amplitude = settings.amplitude_mm;
    let too_dense = "The relief exceeds the 192-motif budget across the selected faces";
    let mut result = Vec::new();
    match &settings.pattern {
        Ribs | Waves => {
            let rows = count(h, pitch)?;
            if rows > limit {
                return Err(too_dense);
            }
            let waves = matches!(settings.pattern, Waves);
            // Four chord segments per wavelength, with wavelength 2 × pitch.
            let segments = ((w / pitch) * 2.).ceil().max(4.);
            if waves && segments > MAX_WAVE_SEGMENTS as f64 {
                return Err(
                    "Waves need a narrower window or larger pitch to fit the 24-segment limit",
                );
            }
            for index in 0..rows {
                let y = center(index, rows, pitch);
                let boundary = if waves {
                    let segments = segments as usize;
                    let x_y = |column: usize, side: f64| {
                        let x = w * column as f64 / segments as f64;
                        let wave = (TAU * x / (2. * pitch)).sin() * pitch * 0.20;
                        [x - w / 2., y + wave + side * pitch * 0.15]
                    };
                    let points: Vec<_> = (0..=segments)
                        .map(|column| x_y(column, -1.))
                        .chain((0..=segments).rev().map(|column| x_y(column, 1.)))
                        .collect();
                    polygon(&points)
                } else {
                    rectangle([0., y], [w, pitch * 0.45])
                };
                result.push(Motif {
                    index,
                    boundary,
                    height_mm: amplitude,
                });
            }
        }
        Stipple => {
            let cols = count(w, pitch)?;
            let rows = count(h, pitch)?;
            if rows * cols > limit {
                return Err(too_dense);
            }
            for row in 0..rows {
                for col in 0..cols {
                    result.push(Motif {
                        index: row * cols + col,
                        boundary: vec![CurvedEdge2d::Circle {
                            center: [center(col, cols, pitch), center(row, rows, pitch)],
                            radius_mm: pitch * 0.22,
                        }],
                        height_mm: amplitude,
                    });
                }
            }
        }
        HeightMap {
            width,
            height,
            samples,
            repeat,
        } => {
            let [width, height] = [usize::from(*width), usize::from(*height)];
            let [cols, rows] = [
                width * usize::from(repeat[0]),
                height * usize::from(repeat[1]),
            ];
            if cols == 0 || rows == 0 || samples.len() != width * height {
                return Err("The retained height-map grid is incomplete");
            }
            let [dx, dy] = [w / cols as f64, h / rows as f64];
            for row in 0..rows {
                for col in 0..cols {
                    let gray = samples[(row % height) * width + col % width];
                    if gray == 0 {
                        continue;
                    }
                    if result.len() == limit {
                        return Err(too_dense);
                    }
                    // Image row zero is the positive V edge.
                    result.push(Motif {
                        index: row * cols + col,
                        boundary: rectangle(
                            [center(col, cols, dx), -center(row, rows, dy)],
                            [0.9 * dx, 0.9 * dy],
                        ),
                        height_mm: amplitude * f64::from(gray) / 255.,
                    });
                }
            }
        }
    }
    if result.is_empty() {
        return Err("The relief pattern contains no nonzero motifs");
    }
    Ok(result)
}

fn shifted(frame: PlaneFrame, station: f64) -> PlaneFrame {
    let normal = frame.normal();
    PlaneFrame {
        origin: std::array::from_fn(|i| frame.origin[i] + normal[i] * station),
        ..frame
    }
}

fn window_frame(
    face: PlaneFrame,
    bounds: [[f64; 2]; 2],
    s: &ResolvedSurfaceStructure,
) -> PlaneFrame {
    let (sin, cos) = s.rotation_rad.sin_cos();
    let [u, v] = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) / 2. + s.offset_mm[i]);
    PlaneFrame {
        origin: face.point([u, v]),
        u: std::array::from_fn(|i| cos * face.u[i] + sin * face.v[i]),
        v: std::array::from_fn(|i| -sin * face.u[i] + cos * face.v[i]),
    }
}

struct Builder<'a> {
    input: FaceProfileInputs<'a>,
    backend: GeometryBackend,
    policy: GeometryPolicy,
    interrupt: Option<&'a dyn Fn() -> bool>,
}

impl Builder<'_> {
    fn fail(&self, message: impl std::fmt::Display) -> ModelError {
        error(Some(self.input.feature), message)
    }

    fn checked<T, E: std::fmt::Display>(&self, value: Result<T, E>) -> Result<T, ModelError> {
        value.map_err(|message| self.fail(message))
    }

    fn prism(
        &self,
        frame: PlaneFrame,
        size: [f64; 2],
        from: f64,
        to: f64,
    ) -> Result<Solid, ModelError> {
        self.extrude(
            &CurvedProfile {
                frame: shifted(frame, from),
                outer: rectangle([0., 0.], size),
                holes: vec![],
            },
            to - from,
            None,
        )
    }

    fn extrude(
        &self,
        profile: &CurvedProfile,
        height: f64,
        origins: Option<&[Origin]>,
    ) -> Result<Solid, ModelError> {
        let transient;
        let origins = match origins {
            Some(origins) => origins,
            None => {
                transient = (0..profile.outer.len()
                    + profile.holes.iter().map(Vec::len).sum::<usize>())
                    .map(|index| Origin {
                        source: self.input.feature,
                        role: format!("surface-check/edge/{index}"),
                    })
                    .collect::<Vec<_>>();
                &transient
            }
        };
        self.checked(match self.backend {
            GeometryBackend::Rust
                if profile
                    .outer
                    .iter()
                    .chain(profile.holes.iter().flatten())
                    .all(|edge| matches!(edge, CurvedEdge2d::Line { .. })) =>
            {
                Solid::rust_extrude_linear_named(
                    profile,
                    profile.frame.normal().map(|v| v * height),
                    0.,
                    self.input.feature,
                    origins,
                    self.policy,
                )
            }
            GeometryBackend::Rust => Solid::rust_extrude_normal_profile_named(
                profile,
                height,
                self.input.feature,
                origins,
                self.policy,
            ),
            GeometryBackend::OpenCascade => Solid::extrude_curved_profile_named(
                profile,
                height,
                self.input.feature,
                origins,
                self.policy,
            ),
        })
    }

    fn boolean(
        &self,
        stock: &Solid,
        tool: &Solid,
        operation: BooleanOperation,
    ) -> Result<Option<Solid>, ModelError> {
        let regular = stock.boolean(tool, operation, self.policy);
        let result = if self.backend == GeometryBackend::Rust
            && regular
                .as_ref()
                .is_err_and(|e| e.kind == GeometryErrorKind::Unsupported)
        {
            match operation {
                BooleanOperation::Subtract => {
                    stock.rust_subtract_planar_tools(&[tool], self.input.feature, self.policy)
                }
                BooleanOperation::Union => {
                    Solid::union_planar_pieces(&[stock, tool], self.input.feature, self.policy)
                        .map(|value| value.map(Some))
                        .unwrap_or(regular)
                }
                BooleanOperation::Intersect => regular,
            }
        } else {
            regular
        };
        self.checked(result)
            .map_err(|cause| self.fail(format!("{operation:?}: {cause}")))
    }

    fn window_fits_profile(
        &self,
        profile: &CurvedProfile,
        bounds: [[f64; 2]; 2],
        settings: &ResolvedSurfaceStructure,
        size: [f64; 2],
    ) -> Result<bool, ModelError> {
        use multicad_kernel::rust_kernel::{
            normal_relief::{Error, Kind, Relief},
            normal_sweep::{Edge, Sweep, signed_area},
        };
        let convert = |curve: &CurvedEdge2d| -> Result<Edge, ModelError> {
            Ok(match *curve {
                CurvedEdge2d::Line { a, b } => Edge::Line { a, b },
                CurvedEdge2d::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                } => Edge::Arc {
                    center,
                    a,
                    b,
                    clockwise,
                },
                CurvedEdge2d::Circle { center, radius_mm } => Edge::Circle {
                    center,
                    radius_mm,
                    clockwise: false,
                },
                CurvedEdge2d::BSpline(_) => return Err(self.fail(
                    "Surface windows on spline boundaries require an exact containment provider",
                )),
            })
        };
        let mut rings = std::iter::once(&profile.outer)
            .chain(&profile.holes)
            .map(|ring| ring.iter().map(convert).collect::<Result<Vec<_>, _>>())
            .collect::<Result<Vec<_>, _>>()?;
        for (index, ring) in rings.iter_mut().enumerate() {
            if (signed_area(ring) > 0.) != (index == 0) {
                ring.reverse();
                for edge in ring {
                    *edge = edge.reversed();
                }
            }
        }
        let (sin, cos) = settings.rotation_rad.sin_cos();
        let center = std::array::from_fn::<_, 2, _>(|i| {
            (bounds[0][i] + bounds[1][i]) / 2. + settings.offset_mm[i]
        });
        let corners = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]].map(|[u, v]| {
            let [x, y] = [u * size[0], v * size[1]];
            [center[0] + cos * x - sin * y, center[1] + sin * x + cos * y]
        });
        let window = corners
            .into_iter()
            .zip(corners.into_iter().cycle().skip(1))
            .map(|(a, b)| Edge::Line { a, b })
            .collect();
        let height = self.policy.minimum_feature_mm.max(1.);
        let tolerance = self.policy.linear_tolerance_mm;
        let minimum = self.policy.minimum_feature_mm;
        let base = Sweep::new(rings, height, tolerance, minimum)
            .map_err(|cause| self.fail(format!("Cannot certify surface boundary: {cause:?}")))?;
        let motif = Sweep::new(vec![window], height, tolerance, minimum)
            .map_err(|cause| self.fail(format!("Cannot certify surface window: {cause:?}")))?;
        // The shared finite-cap certificate proves the complete padded window
        // is strictly inside the stock and disjoint from every existing hole.
        // Its separate combined-edge budget includes a maximum-size source.
        match Relief::new_with_interrupt(
            base,
            vec![motif],
            Kind::Raised,
            tolerance,
            minimum,
            self.interrupt,
        ) {
            Ok(_) => Ok(true),
            Err(Error::Cancelled) => Err(self.fail("Surface window was superseded")),
            Err(Error::Limit) => {
                Err(self.fail("Surface window containment exceeds the certified work budget"))
            }
            Err(Error::Invalid | Error::Unresolved) => Ok(false),
        }
    }

    fn covered(
        &self,
        required: &Solid,
        container: &Solid,
        message: &str,
    ) -> Result<(), ModelError> {
        checkpoint(self.interrupt)?;
        if self
            .boolean(required, container, BooleanOperation::Subtract)
            .map_err(|cause| self.fail(format!("{message} ({cause})")))?
            .is_some()
        {
            return Err(self.fail(message));
        }
        Ok(())
    }

    fn disjoint(&self, a: &Solid, b: &Solid, message: &str) -> Result<(), ModelError> {
        checkpoint(self.interrupt)?;
        if self.boolean(a, b, BooleanOperation::Intersect)?.is_some() {
            return Err(self.fail(message));
        }
        Ok(())
    }

    fn tool(
        &self,
        frame: PlaneFrame,
        face: EntityId,
        motif: &Motif,
        start: f64,
        end: f64,
    ) -> Result<Solid, ModelError> {
        checkpoint(self.interrupt)?;
        let id = self.input.feature;
        let role = format!("surface/{face}/{}", motif.index);
        let component = self.checked(EntityId::parse(
            &ContentHash::of(role.as_bytes()).to_string()[..32],
        ))?;
        let origins = (0..motif.boundary.len())
            .map(|edge| Origin {
                source: id,
                role: format!("{role}/edge/{edge}"),
            })
            .collect::<Vec<_>>();
        self.extrude(
            &CurvedProfile {
                frame: shifted(frame, start),
                outer: motif.boundary.clone(),
                holes: vec![],
            },
            end - start,
            Some(&origins),
        )?
        .with_component_origin(id, component)
        .map_err(|e| self.fail(e))
    }
}

pub(crate) fn build(
    input: FaceProfileInputs<'_>,
    parameters: &EvaluatedParameters,
    backend: GeometryBackend,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    let builder = Builder {
        policy: GeometryPolicy {
            boolean_fuzzy_mm: 0.,
            ..input.policy
        },
        input,
        backend,
        interrupt,
    };
    checkpoint(interrupt)?;
    let id = builder.input.feature;
    let source = builder.checked(model::read_surface_structure(builder.input.document, id))?;
    let settings = builder.checked(source.parameters.resolve(parameters))?;
    let stock = builder
        .input
        .solids
        .get(&source.input.producer)
        .ok_or_else(|| {
            builder.fail("The retained relief predecessor has no current native solid")
        })?;
    if builder.checked(stock.solid_count())? != 1 {
        return Err(builder.fail("Surface Structure needs one connected receiving solid"));
    }
    let single_face = source.faces.len() == 1;
    let pattern = builder.checked(motifs(
        &settings,
        MAX_SURFACE_STRUCTURE_MOTIFS / source.faces.len(),
    ))?;
    let policy = builder.policy;
    if pattern
        .iter()
        .any(|motif| motif.height_mm < policy.minimum_feature_mm)
    {
        return Err(builder.fail("A nonzero motif height is below the native minimum feature. Increase the physical relief height."));
    }
    let raised = settings.direction == SurfaceStructureDirection::Raised;
    let floor = MIN_RECESS_FLOOR_MM.max(policy.minimum_feature_mm);
    let bond = policy
        .minimum_feature_mm
        .max(32. * policy.linear_tolerance_mm);
    let mut swept_windows = Vec::with_capacity(source.faces.len());
    let mut tools = Vec::with_capacity(source.faces.len() * pattern.len());
    for face in source.faces {
        checkpoint(interrupt)?;
        let resolved = builder.input.resolve_planar(face.id)?;
        let frame = window_frame(resolved.profile.frame, resolved.bounds_mm, &settings);
        let size = settings.size_mm;
        let padded = size.map(|value| value + 2. * settings.border_mm);
        if builder.backend == GeometryBackend::Rust {
            if !builder.window_fits_profile(
                &resolved.profile,
                resolved.bounds_mm,
                &settings,
                padded,
            )? {
                return Err(builder.fail("The relief window and border cross the selected face boundary or an opening. Reduce the size or move the window."));
            }
        } else {
            let envelope = builder.extrude(&resolved.profile, floor, Some(&resolved.origins))?;
            let border = builder.prism(frame, padded, 0., floor)?;
            builder.covered(
                &border,
                &envelope,
                "The relief window and border cross the selected face boundary or an opening. Reduce the size or move the window.",
            )?;
        }
        let (support, exterior, from, to) = if raised {
            (
                bond,
                settings.amplitude_mm,
                -2. * bond,
                settings.amplitude_mm + bond,
            )
        } else {
            (
                settings.amplitude_mm + floor,
                bond,
                -settings.amplitude_mm - floor - bond,
                2. * bond,
            )
        };
        if let Some(depth) =
            builder.checked(stock.rust_prismatic_cap_depth(resolved.profile.frame))?
        {
            if depth < support {
                return Err(builder.fail(if raised {
                    "The raised window does not have continuous native material for attachment"
                } else {
                    "The recess would break through or cross an internal void. Reduce the depth to retain a 0.10 mm floor."
                }));
            }
        } else {
            let required_stock = builder.prism(frame, size, -support, 0.)?;
            builder.covered(
                &required_stock,
                stock,
                if raised {
                    "The raised window does not have continuous native material for attachment"
                } else {
                    "The recess would break through or cross an internal void. Reduce the depth to retain a 0.10 mm floor."
                },
            )?;
            let outside = builder.prism(frame, size, 0., exterior)?;
            builder.disjoint(
                &outside,
                stock,
                "The relief window crosses existing material outside the selected face",
            )?;
        }
        if single_face && builder.backend == GeometryBackend::Rust {
            let (sin, cos) = settings.rotation_rad.sin_cos();
            let center = std::array::from_fn::<_, 2, _>(|i| {
                (resolved.bounds_mm[0][i] + resolved.bounds_mm[1][i]) / 2. + settings.offset_mm[i]
            });
            let point =
                |[x, y]: [f64; 2]| [center[0] + cos * x - sin * y, center[1] + sin * x + cos * y];
            let transformed = pattern
                .iter()
                .map(|motif| {
                    let boundary = motif
                        .boundary
                        .iter()
                        .map(|edge| {
                            Ok(match edge {
                                CurvedEdge2d::Line { a, b } => CurvedEdge2d::Line {
                                    a: point(*a),
                                    b: point(*b),
                                },
                                CurvedEdge2d::Arc {
                                    center,
                                    a,
                                    b,
                                    clockwise,
                                } => CurvedEdge2d::Arc {
                                    center: point(*center),
                                    a: point(*a),
                                    b: point(*b),
                                    clockwise: *clockwise,
                                },
                                CurvedEdge2d::Circle { center, radius_mm } => {
                                    CurvedEdge2d::Circle {
                                        center: point(*center),
                                        radius_mm: *radius_mm,
                                    }
                                }
                                CurvedEdge2d::BSpline(_) => return Err(builder.fail(
                                    "Spline relief motifs require an exact construction provider",
                                )),
                            })
                        })
                        .collect::<Result<Vec<_>, ModelError>>()?;
                    Ok(NormalReliefMotif {
                        boundary,
                        height_mm: motif.height_mm,
                        origin: Origin {
                            source: id,
                            role: format!("surface/{}/{}", face.id, motif.index),
                        },
                    })
                })
                .collect::<Result<Vec<_>, ModelError>>()?;
            match stock.rust_normal_relief_on_cap(
                resolved.face_index,
                resolved.profile.frame,
                &transformed,
                if raised {
                    multicad_kernel::rust_kernel::normal_relief::Kind::Raised
                } else {
                    multicad_kernel::rust_kernel::normal_relief::Kind::Recessed
                },
                id,
                policy,
                interrupt,
            ) {
                Ok(result) => return Ok(result),
                Err(cause) if cause.kind == GeometryErrorKind::Unsupported => {}
                Err(cause) => return Err(builder.fail(cause)),
            }
        }
        let swept = builder.prism(frame, padded, from, to)?;
        for other in &swept_windows {
            builder.disjoint(
                &swept, other,
                "Selected face relief windows overlap in 3D. Leave a margin between the structures.",
            )?;
        }
        swept_windows.push(swept);
        for motif in &pattern {
            let (from, to) = if raised {
                (-bond, motif.height_mm)
            } else {
                (bond, -motif.height_mm)
            };
            tools.push(builder.tool(frame, face.id, motif, from, to)?);
        }
    }
    let operation = if raised {
        BooleanOperation::Union
    } else {
        BooleanOperation::Subtract
    };
    let before = builder.checked(stock.volume_mm3())?;
    let mut result = None;
    // Positive gaps between motifs let exact compounds batch independent prisms.
    // Bound each Boolean so cancellation has checkpoints between native calls.
    for chunk in tools.chunks(32) {
        checkpoint(interrupt)?;
        let receiver = result.as_ref().unwrap_or(stock);
        let next = if builder.backend == GeometryBackend::Rust {
            let regular = match operation {
                BooleanOperation::Union => receiver.rust_union_orthogonal_tools(chunk, policy),
                BooleanOperation::Subtract => {
                    receiver.rust_subtract_orthogonal_tools(chunk, policy)
                }
                BooleanOperation::Intersect => unreachable!(),
            };
            let evaluated = if regular
                .as_ref()
                .is_err_and(|e| e.kind == GeometryErrorKind::Unsupported)
            {
                let members = chunk.iter().collect::<Vec<_>>();
                match operation {
                    BooleanOperation::Subtract => {
                        receiver.rust_subtract_planar_tools(&members, id, policy)
                    }
                    BooleanOperation::Union => {
                        let inputs = std::iter::once(receiver).chain(members).collect::<Vec<_>>();
                        Solid::union_planar_pieces(&inputs, id, policy)
                            .map(|v| v.map(Some))
                            .unwrap_or(regular)
                    }
                    BooleanOperation::Intersect => unreachable!(),
                }
            } else {
                regular
            };
            builder.checked(evaluated)?.ok_or_else(|| {
                builder
                    .fail("Surface Structure removed the whole Part or produced no valid geometry")
            })?
        } else {
            let compound;
            let tool = if chunk.len() == 1 {
                &chunk[0]
            } else {
                compound = builder.checked(Solid::compound_named_interruptible(
                    &chunk.iter().collect::<Vec<_>>(),
                    id,
                    policy,
                    interrupt,
                ))?;
                &compound
            };
            builder
                .boolean(result.as_ref().unwrap_or(stock), tool, operation)?
                .ok_or_else(|| {
                    builder.fail(
                        "Surface Structure removed the whole Part or produced no valid geometry",
                    )
                })?
        };
        if builder.checked(next.solid_count())? != 1 {
            return Err(builder.fail("Surface Structure must preserve one connected solid"));
        }
        result = Some(next);
    }
    checkpoint(interrupt)?;
    let result = result.ok_or_else(|| builder.fail("Surface Structure produced no tools"))?;
    let after = builder.checked(result.volume_mm3())?;
    if (if raised {
        after - before
    } else {
        before - after
    }) <= policy.minimum_feature_mm.powi(3)
    {
        return Err(builder.fail("Surface Structure did not make a verifiable material change"));
    }
    Ok(result)
}
