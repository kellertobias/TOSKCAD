//! Inset flanges change the section along their span. Join their exact
//! root boundaries to one relieved web rather than fusing display solids.
use super::*;
use crate::sheet_forming::{FlatFrame, section, shape_edges};
use multicad_core::references::{AnchoredFrame, ReferenceFrame};
use multicad_kernel::rust_kernel::{
    normal_sweep::{Edge, signed_area},
    warped_union::{SheetFlange, SheetFlangeOpening, SheetWebEdge, formed_sheet_with_openings},
};
use std::collections::BTreeMap;

pub(super) fn build(layout: &mut SheetLayout, policy: GeometryPolicy) -> Result<Solid, String> {
    let thickness = layout.thickness_mm;
    let count = layout.flanges.len();
    if count > 2
        || (count == 2
            && layout.flanges[0]
                .flange
                .edge
                .abs_diff(layout.flanges[1].flange.edge)
                != 2)
        || layout.flanges.iter().any(|f| {
            !matches!(
                f.flange.segments.as_slice(),
                [StripSegment::Bend { .. }, StripSegment::Straight { .. }]
            )
        })
    {
        return Err("Rust inset formed sheet requires one bend or two opposite bends".into());
    }
    let mut origins = Vec::new();
    let feature = layout.forming_feature;
    let authored = |role| Origin {
        source: feature,
        role,
    };
    let mut source = |origin| {
        origins.push(origin);
        origins.len() - 1
    };
    let mut outer = Vec::new();
    let mut developed = Vec::new();
    for (index, edge) in layout.base.outer.iter().enumerate() {
        let CurvedEdge2d::Line { a, b } = *edge else {
            return Err("Inset formed sheet needs straight web edges".into());
        };
        let Some((hinge, f)) = layout
            .flanges
            .iter()
            .enumerate()
            .find(|(_, f)| f.flange.edge == index)
        else {
            outer.push(SheetWebEdge {
                ends: [a, b],
                source: source(authored(format!("sheet.base.edge.{index}"))),
                flange: None,
            });
            developed.push(PlanarEdge::Line { a, b });
            continue;
        };
        let points = edge_points(f, a, b, 0.)?;
        let root = [point(f, 0., 0.), point(f, f.width, 0.)];
        for (i, pair) in points.windows(2).enumerate() {
            outer.push(SheetWebEdge {
                ends: [pair[0], pair[1]],
                source: source(authored(format!("sheet.base.edge.{index}.segment.{i}"))),
                flange: (pair == root).then_some(hinge),
            });
        }
        let extension = f.flange.developed_mm(thickness, layout.k_factor);
        developed.extend(edge_points(f, a, b, extension)?.windows(2).map(|pair| {
            PlanarEdge::Line {
                a: pair[0],
                b: pair[1],
            }
        }));
    }
    let mut loops = vec![PlanarLoop {
        outer: true,
        edges: developed,
    }];
    let mut holes = Vec::new();
    let mut flange_cuts: Vec<Vec<Opening>> = (0..count).map(|_| Vec::new()).collect();
    let flange_index = |index| {
        layout
            .flanges
            .iter()
            .position(|f| f.flange.index == index as usize)
            .ok_or_else(|| format!("Flange {} does not exist", index + 1))
    };
    for cut in &layout.cutouts {
        let name = cut.source.id();
        let flat = flat_cutout(cut, &layout.flanges, thickness, layout.k_factor)
            .ok_or_else(|| format!("{name} names a missing flange"))?;
        loops.push(channel::developed_hole(&flat)?);
        let outline = shape_edges(&cut.shape);
        let ring = outline
            .iter()
            .enumerate()
            .map(|(i, edge)| {
                Ok((
                    profile_edge(edge)?,
                    source(authored(format!("sheet.{name}.edge.{i}"))),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        match cut.face {
            CutoutFace::Base => holes.push(oriented(ring, false)),
            CutoutFace::Flange { index } => {
                flange_cuts[flange_index(index)?].push(Opening::new(layout, cut.face, ring)?)
            }
        }
    }
    for cut in &layout.profile_cuts {
        let flange = match cut.face {
            CutoutFace::Base => None,
            CutoutFace::Flange { index } => Some(flange_index(index)?),
        };
        let outline = if let Some(index) = flange {
            let f = &layout.flanges[index];
            let flat = FlatFrame::flange(f, f.flange.leg_offset_mm(thickness, layout.k_factor));
            super::profile_cut::transform(&cut.outline, |p| flat.coordinates(p), flat.reflected())?
        } else {
            cut.outline.clone()
        };
        if outline.len() != cut.origins.len() {
            return Err("Inset sheet cut needs one authored origin per edge".into());
        }
        let ring = outline
            .iter()
            .zip(&cut.origins)
            .map(|(edge, origin)| Ok((profile_edge(edge)?, source(origin.clone()))))
            .collect::<Result<Vec<_>, String>>()?;
        if let Some(index) = flange {
            flange_cuts[index].push(Opening::new(layout, cut.face, ring)?);
        } else {
            holes.push(oriented(ring, false));
        }
        loops.push(channel::developed_hole(&cut.outline)?);
    }
    let mut authored_frames = BTreeMap::new();
    let thickness_side = dot(layout.frame.normal(), layout.thickness_direction);
    let sections = layout
        .flanges
        .iter()
        .zip(&flange_cuts)
        .map(|(f, cuts)| {
            let profile = section(&f.flange, thickness);
            let edges = profile
                .edges
                .iter()
                .zip(&profile.roles)
                .map(|(edge, role)| {
                    let source = source(authored(role.clone()));
                    if let CurvedEdge2d::Line { a, b } = *edge
                        && (role.ends_with(".leg.a") || role.ends_with(".leg.b"))
                    {
                        let position = |[outward, height]: [f64; 2]| {
                            [
                                f.start[0] + f.outward[0] * outward,
                                f.start[1] + f.outward[1] * outward,
                                thickness_side * height,
                            ]
                        };
                        let [a, b] = [position(a), position(b)];
                        authored_frames.insert(
                            source,
                            AnchoredFrame {
                                origin: authored(format!("side/{feature}/{role}")),
                                frame: ReferenceFrame {
                                    origin_mm: a,
                                    u: unit(std::array::from_fn(|i| b[i] - a[i])),
                                    v: [f.along[0], f.along[1], 0.],
                                },
                            },
                        );
                    }
                    Ok((profile_edge(edge)?, source))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let caps = ["start", "end"].map(|end| {
                source(authored(format!(
                    "sheet.flange.{}.cap.{end}",
                    f.flange.index
                )))
            });
            let mut openings = Vec::new();
            if !cuts.is_empty() {
                let find_wall = |side| {
                    let role = format!("sheet.flange.{}.leg.{side}", f.flange.index);
                    profile
                        .roles
                        .iter()
                        .position(|candidate| candidate == &role)
                        .ok_or("An inset flange opening needs two complete leg walls")
                };
                let walls = [find_wall("a")?, find_wall("b")?];
                openings.extend(cuts.iter().map(|cut| cut.as_sheet_opening(walls)));
            }
            Ok((edges, caps, openings))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let flanges = sections
        .iter()
        .map(|(section, ends, _)| SheetFlange {
            section,
            ends: *ends,
        })
        .collect::<Vec<_>>();
    let openings: Vec<_> = sections
        .iter()
        .map(|(_, _, cuts)| cuts.as_slice())
        .collect();
    let web_caps = [channel::WEB_A, channel::WEB_B].map(|role| source(authored(role.into())));
    let shell = formed_sheet_with_openings(
        &outer,
        &holes,
        web_caps,
        &flanges,
        &openings,
        thickness.copysign(thickness_side),
        policy.linear_tolerance_mm,
        policy.minimum_feature_mm,
    )
    .map_err(|cause| format!("The inset formed sheet boundary cannot be certified: {cause:?}"))?;
    let solid = Solid::rust_place_formed_sheet(
        shell,
        layout.frame,
        layout.forming_feature,
        &origins,
        web_caps,
        &authored_frames,
        policy,
    )
    .map_err(|cause| cause.to_string())?;
    let contours = PlanarContours {
        frame: ReferenceFrame {
            origin_mm: layout.frame.origin,
            u: layout.frame.u,
            v: layout.frame.v,
        },
        loops,
    };
    contours.validate(policy.linear_tolerance_mm)?;
    layout.developed = Some(contours);
    Ok(solid)
}

type Ring = Vec<(Edge, usize)>;

fn oriented(mut ring: Ring, ccw: bool) -> Ring {
    let edges: Vec<_> = ring.iter().map(|&(edge, _)| edge).collect();
    if (signed_area(&edges) > 0.) != ccw {
        ring.reverse();
        ring.iter_mut()
            .for_each(|(edge, _)| *edge = edge.reversed());
    }
    ring
}

struct Opening {
    frame: PlaneFrame,
    through_mm: [f64; 3],
    outline: Ring,
}

impl Opening {
    fn new(layout: &SheetLayout, face: CutoutFace, outline: Ring) -> Result<Self, String> {
        let (frame, normal) = layout.space().face_frame(face, &layout.flanges)?;
        let axes = [layout.frame.u, layout.frame.v, layout.frame.normal()];
        let local = |vector| axes.map(|axis| dot(vector, axis));
        Ok(Self {
            frame: PlaneFrame {
                origin: local(std::array::from_fn(|i| {
                    frame.origin[i] - layout.frame.origin[i]
                })),
                u: local(frame.u),
                v: local(frame.v),
            },
            through_mm: local(scale(normal, layout.thickness_mm)),
            outline: oriented(outline, true),
        })
    }

    fn as_sheet_opening(&self, walls: [usize; 2]) -> SheetFlangeOpening<'_> {
        SheetFlangeOpening {
            walls,
            origin_mm: self.frame.origin,
            u: self.frame.u,
            v: self.frame.v,
            through_mm: self.through_mm,
            outline: &self.outline,
        }
    }
}

fn profile_edge(edge: &CurvedEdge2d) -> Result<Edge, String> {
    match *edge {
        CurvedEdge2d::Line { a, b } => Ok(Edge::Line { a, b }),
        CurvedEdge2d::Arc {
            center,
            a,
            b,
            clockwise,
        } => Ok(Edge::Arc {
            center,
            a,
            b,
            clockwise,
        }),
        CurvedEdge2d::Circle { center, radius_mm } => Ok(Edge::Circle {
            center,
            radius_mm,
            clockwise: true,
        }),
        CurvedEdge2d::BSpline(_) => {
            Err("Inset formed sheet requires straight or circular boundaries".into())
        }
    }
}

fn point(f: &PlacedFlange, along: f64, out: f64) -> [f64; 2] {
    std::array::from_fn(|i| f.start[i] + f.along[i] * along + f.outward[i] * out)
}

/// One web edge and its reliefs, with optional outward flat extension.
pub(super) fn edge_points(
    f: &PlacedFlange,
    a: [f64; 2],
    b: [f64; 2],
    extension: f64,
) -> Result<Vec<[f64; 2]>, String> {
    let at = |along, out| point(f, along, out);
    let mut points = vec![a];
    let relief = f.flange.relief_mm;
    if f.flange.start_inset_mm > 1e-9 {
        if let Some([w, depth]) = relief {
            if w > f.flange.start_inset_mm + 1e-9 {
                return Err(format!(
                    "Flange {} relief is wider than its start inset",
                    f.flange.index + 1
                ));
            }
            points.extend([at(-w, 0.), at(-w, -depth), at(0., -depth)]);
        }
        points.push(at(0., 0.));
    }
    points.extend([at(0., extension), at(f.width, extension)]);
    if f.flange.end_inset_mm > 1e-9 {
        points.push(at(f.width, 0.));
        if let Some([w, depth]) = relief {
            if w > f.flange.end_inset_mm + 1e-9 {
                return Err(format!(
                    "Flange {} relief is wider than its end inset",
                    f.flange.index + 1
                ));
            }
            points.extend([
                at(f.width, -depth),
                at(f.width + w, -depth),
                at(f.width + w, 0.),
            ]);
        }
    }
    points.push(b);
    points.dedup_by(|p, q| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-9);
    Ok(points)
}
