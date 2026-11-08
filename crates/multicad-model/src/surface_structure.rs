//! Bounded, exact native relief on selected finite planar faces.
use crate::{
    ModelError,
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
    BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, PlaneFrame, Solid,
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

fn window_prism(
    frame: PlaneFrame,
    size: [f64; 2],
    from: f64,
    to: f64,
    policy: GeometryPolicy,
) -> Result<Solid, multicad_kernel::GeometryError> {
    Solid::extrude_curved_profile(
        &CurvedProfile {
            frame: shifted(frame, from),
            outer: rectangle([0., 0.], size),
            holes: vec![],
        },
        to - from,
        policy,
    )
}

struct Builder<'a> {
    input: FaceProfileInputs<'a>,
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
        self.checked(window_prism(frame, size, from, to, self.policy))
    }

    fn covered(
        &self,
        required: &Solid,
        container: &Solid,
        message: &str,
    ) -> Result<(), ModelError> {
        checkpoint(self.interrupt)?;
        if self
            .checked(required.boolean(container, BooleanOperation::Subtract, self.policy))?
            .is_some()
        {
            return Err(self.fail(message));
        }
        Ok(())
    }

    fn disjoint(&self, a: &Solid, b: &Solid, message: &str) -> Result<(), ModelError> {
        checkpoint(self.interrupt)?;
        if self
            .checked(a.boolean(b, BooleanOperation::Intersect, self.policy))?
            .is_some()
        {
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
        self.checked(Solid::extrude_curved_profile_named(
            &CurvedProfile {
                frame: shifted(frame, start),
                outer: motif.boundary.clone(),
                holes: vec![],
            },
            end - start,
            id,
            &origins,
            self.policy,
        ))?
        .with_component_origin(id, component)
        .map_err(|e| self.fail(e))
    }
}

pub(crate) fn build(
    input: FaceProfileInputs<'_>,
    parameters: &EvaluatedParameters,
    interrupt: Option<&dyn Fn() -> bool>,
) -> Result<Solid, ModelError> {
    let builder = Builder {
        policy: GeometryPolicy {
            boolean_fuzzy_mm: 0.,
            ..input.policy
        },
        input,
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
        let envelope = builder.checked(Solid::extrude_curved_profile(
            &resolved.profile,
            floor,
            policy,
        ))?;
        let border = builder.prism(frame, padded, 0., floor)?;
        builder.covered(
            &border,
            &envelope,
            "The relief window and border cross the selected face boundary or an opening. Reduce the size or move the window.",
        )?;
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
        let next = builder
            .checked(
                result
                    .as_ref()
                    .unwrap_or(stock)
                    .boolean(tool, operation, policy),
            )?
            .ok_or_else(|| {
                builder
                    .fail("Surface Structure removed the whole Part or produced no valid geometry")
            })?;
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
