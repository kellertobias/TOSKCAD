use super::{
    SheetLayout, channel, cross, dot, flat_contours, polygon_edges, rebuild_rust_flat, scale,
};
use crate::{GeometryBackend, ModelError, error, sheet_forming::FlatFrame};
use multicad_core::{document::EntityId, model::CutoutFace, references::Origin};
use multicad_kernel::{BooleanOperation, CurvedEdge2d, CurvedProfile, GeometryPolicy, Solid};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProfileCut {
    pub feature: EntityId,
    pub face: CutoutFace,
    pub outline: Vec<CurvedEdge2d>,
    pub area_mm2: f64,
    pub origins: Vec<Origin>,
}

pub(crate) struct CutProfile {
    pub geometry: CurvedProfile,
    pub developed: bool,
    pub backend: GeometryBackend,
}

pub(crate) fn build_profile_cut(
    previous: &SheetLayout,
    sheet: &Solid,
    feature: EntityId,
    face: CutoutFace,
    profile: CutProfile,
    origins: &[Origin],
    policy: GeometryPolicy,
) -> Result<(Solid, SheetLayout), ModelError> {
    let CutProfile {
        geometry: profile,
        developed,
        backend,
    } = profile;
    let failure = |message: String| error(Some(feature), message);
    let native = |e: multicad_kernel::GeometryError| failure(e.to_string());
    if !profile.holes.is_empty() {
        return Err(failure(
            "Choose one closed region without internal islands for a sheet cut".into(),
        ));
    }

    let (frame, normal) = previous
        .space()
        .face_frame(face, &previous.flanges)
        .map_err(failure)?;
    let thickness = previous.thickness_mm;
    let flange = match face {
        CutoutFace::Base => None,
        CutoutFace::Flange { index } => Some(
            previous
                .flanges
                .iter()
                .find(|flange| flange.flange.index == index as usize)
                .ok_or_else(|| failure(format!("Flange {} is missing", index + 1)))?,
        ),
    };
    let flat_frame = flange.map_or(FlatFrame::BASE, |flange| {
        FlatFrame::flange(
            flange,
            flange.flange.leg_offset_mm(thickness, previous.k_factor),
        )
    });
    let (outer, outline) = if developed {
        let sketch = profile.frame;
        let sketch_normal = sketch.normal();
        if sketch_normal[2].abs() < 1. - 1e-9 || sketch.origin[2].abs() > policy.linear_tolerance_mm
        {
            return Err(failure(
                "Keep the sketch on the developed sheet plane".into(),
            ));
        }
        let outline = transform(
            &profile.outer,
            |p| {
                let [x, y, _] = sketch.point(p);
                [x, y]
            },
            sketch_normal[2] < 0.,
        )
        .map_err(failure)?;
        let outer = transform(
            &outline,
            |p| flat_frame.coordinates(p),
            flat_frame.reflected(),
        )
        .map_err(failure)?;
        (outer, outline)
    } else {
        let delta: [f64; 3] = std::array::from_fn(|i| profile.frame.origin[i] - frame.origin[i]);
        let offset = dot(delta, normal);
        let sketch_normal = profile.frame.normal();
        let tolerance = policy.linear_tolerance_mm;
        if dot(sketch_normal, normal).abs() < 1. - 1e-9
            || (offset.abs() > tolerance && (offset - thickness).abs() > tolerance)
        {
            return Err(failure(
                "Place the sketch directly on the flat base or final flange leg being cut".into(),
            ));
        }

        let map = |p: [f64; 2]| {
            let point = std::array::from_fn(|i| {
                delta[i] + p[0] * profile.frame.u[i] + p[1] * profile.frame.v[i]
            });
            [dot(point, frame.u), dot(point, frame.v)]
        };
        let reflected = dot(sketch_normal, cross(frame.u, frame.v)) < 0.;
        let outer = transform(&profile.outer, map, reflected).map_err(failure)?;
        let outline =
            transform(&outer, |p| flat_frame.point(p), flat_frame.reflected()).map_err(failure)?;
        (outer, outline)
    };
    if backend == GeometryBackend::Rust {
        let mut layout = previous.clone();
        layout.profile_cuts.push(ProfileCut {
            feature,
            face,
            outline,
            area_mm2: 0.,
            origins: origins.to_vec(),
        });
        let cut = if layout.flanges.is_empty() {
            rebuild_rust_flat(&mut layout, feature, policy)?
        } else {
            let (solid, developed) = channel::append_profile(
                previous,
                sheet,
                outer,
                layout
                    .profile_cuts
                    .last()
                    .expect("newly appended profile cut"),
                policy,
            )
            .map_err(failure)?;
            layout.developed = Some(developed);
            solid
        };
        let removed = sheet.volume_mm3().map_err(native)? - cut.volume_mm3().map_err(native)?;
        if removed <= 0. || !removed.is_finite() {
            return Err(failure(
                "The exact cut does not remove material from the sheet".into(),
            ));
        }
        layout
            .profile_cuts
            .last_mut()
            .expect("newly appended profile cut")
            .area_mm2 = removed / thickness;
        return Ok((cut, layout));
    }
    let support = flange.map_or_else(
        || previous.base.outer.clone(),
        |flange| {
            let (width, leg) = (flange.width, flange.flange.leg_mm);
            polygon_edges(&[[0., 0.], [width, 0.], [width, leg], [0., leg]])
        },
    );
    // Prove that the entire analytic loop lies within the one supported face.
    // The folded volume check below separately catches existing openings.
    CurvedProfile {
        frame,
        outer: support,
        holes: vec![outer.clone()],
    }
    .area_mm2(policy)
    .map_err(|e| {
        failure(format!(
            "Keep the entire cut inside one flat sheet face, clear of its edges and bends: {e}"
        ))
    })?;
    let tool = Solid::extrude_curved_profile_vector_named(
        &CurvedProfile {
            frame,
            outer: outer.clone(),
            holes: vec![],
        },
        scale(normal, thickness),
        feature,
        origins,
        policy,
    )
    .map_err(native)?;
    let before = sheet.volume_mm3().map_err(native)?;
    let cut = sheet
        .boolean(&tool, BooleanOperation::Subtract, policy)
        .map_err(native)?
        .ok_or_else(|| failure("The cut removes the entire sheet".into()))?;
    let expected = tool.volume_mm3().map_err(native)?;
    let removed = before - cut.volume_mm3().map_err(native)?;
    if (removed - expected).abs() > 1e-6 * expected.max(1.) {
        return Err(failure(
            "The cut overlaps an existing opening or leaves the selected flat sheet face".into(),
        ));
    }
    if cut.solid_count().map_err(native)? != 1 {
        return Err(failure(
            "The cut splits the sheet into separate pieces".into(),
        ));
    }

    let mut layout = previous.clone();
    layout.profile_cuts.push(ProfileCut {
        feature,
        face,
        outline,
        area_mm2: expected / thickness,
        origins: origins.to_vec(),
    });
    layout.developed = None;
    layout.developed = Some(
        flat_contours(&layout, policy)
            .map_err(|e| failure(format!("Flat development failed: {e}")))?,
    );
    Ok((cut, layout))
}

fn transform(
    edges: &[CurvedEdge2d],
    map: impl Fn([f64; 2]) -> [f64; 2],
    reflected: bool,
) -> Result<Vec<CurvedEdge2d>, String> {
    edges
        .iter()
        .map(|edge| {
            Ok(match *edge {
                CurvedEdge2d::Line { a, b } => CurvedEdge2d::Line {
                    a: map(a),
                    b: map(b),
                },
                CurvedEdge2d::Arc {
                    a,
                    b,
                    center,
                    clockwise,
                } => CurvedEdge2d::Arc {
                    a: map(a),
                    b: map(b),
                    center: map(center),
                    clockwise: clockwise ^ reflected,
                },
                CurvedEdge2d::Circle { center, radius_mm } => CurvedEdge2d::Circle {
                    center: map(center),
                    radius_mm,
                },
                CurvedEdge2d::BSpline(_) => {
                    return Err("Use lines, arcs or circles in a developable sheet cut".into());
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use multicad_core::{model, project_file};

    #[test]
    fn c_channel_refuses_crossing_the_bend_and_recurring_openings() {
        let document = project_file::read(
            &mut include_bytes!("../../../../examples/front-panel.tcd").as_slice(),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap()
        .project
        .document;
        let part = document
            .entities()
            .find(|entity| entity.name == "Front panel sheet")
            .unwrap()
            .id;
        let forming = model::sheet_feature(&document, part).unwrap();
        let compiler = crate::Compiler {
            omit_display: true,
            ..Default::default()
        };
        let model = compiler.compile(&document, 0).unwrap();
        let layout = &model.snapshot().sheet_layouts[&forming];
        let sheet = model.solid(forming).unwrap();
        let before = sheet.volume_mm3().unwrap();
        let feature = EntityId::new();
        let refuse = |face, outline: Vec<_>| {
            let (frame, _) = layout.space().face_frame(face, &layout.flanges).unwrap();
            let origins: Vec<_> = (0..outline.len())
                .map(|index| Origin {
                    source: feature,
                    role: format!("edge.{index}"),
                })
                .collect();
            let result = build_profile_cut(
                layout,
                sheet,
                feature,
                face,
                CutProfile {
                    geometry: CurvedProfile {
                        frame,
                        outer: outline,
                        holes: vec![],
                    },
                    developed: false,
                    backend: GeometryBackend::OpenCascade,
                },
                &origins,
                compiler.policy,
            );
            result.err().expect("Invalid cut must fail").message
        };
        let crosses = refuse(
            CutoutFace::Flange { index: 0 },
            polygon_edges(&[[41., -1.], [49., -1.], [49., 5.], [41., 5.]]),
        );
        assert!(
            crosses.contains("clear of its edges and bends"),
            "{crosses}"
        );
        let opening = layout
            .cutouts
            .iter()
            .find(|cutout| cutout.face == CutoutFace::Base)
            .unwrap();
        let overlaps = refuse(
            opening.face,
            crate::sheet_forming::shape_edges(&opening.shape),
        );
        assert!(
            overlaps.contains("overlaps an existing opening"),
            "{overlaps}"
        );
        assert_eq!(sheet.volume_mm3().unwrap(), before);
    }
}
