//! Exact nominal purchased pieces on a resolved sheet face.

use super::*;
use multicad_core::references::Origin;
use multicad_kernel::RevolutionAxis;
use std::f64::consts::TAU;

pub(super) struct Mount {
    pub frame: PlaneFrame,
    pub surface: [f64; 3],
    pub rear: [f64; 3],
    pub out: [f64; 3],
}

impl Mount {
    fn frame_at(&self, origin: [f64; 3], [x, y, z]: [f64; 3]) -> PlaneFrame {
        let [a, b, c] = self.frame.u;
        PlaneFrame {
            origin,
            u: self.frame.u,
            v: [y * c - z * b, z * a - x * c, x * b - y * a],
        }
    }

    fn hex_frame(&self, origin: [f64; 3], axis: [f64; 3]) -> PlaneFrame {
        let frame = self.frame_at(origin, axis);
        // The sheet hexagon has a vertex on +u. The shared Rust fastener
        // primitive places a flat at +u, so rotate its frame by 30 degrees.
        let cosine = 3_f64.sqrt() / 2.;
        PlaneFrame {
            origin,
            u: std::array::from_fn(|i| cosine * frame.u[i] - 0.5 * frame.v[i]),
            v: std::array::from_fn(|i| 0.5 * frame.u[i] + cosine * frame.v[i]),
        }
    }
}

fn round_head(
    mount: &Mount,
    seat: [f64; 3],
    back: [f64; 3],
    dimensions: [f64; 4],
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<Solid, String> {
    let [head_radius, head_height, shaft_radius, length] = dimensions;
    // One closed meridian gives the same continuous solid as the nominal
    // head and shaft. It has no internal cap at the head/shaft junction.
    let meridian = [
        [0., -head_height],
        [head_radius, -head_height],
        [head_radius, 0.],
        [shaft_radius, 0.],
        [shaft_radius, length],
        [0., length],
    ];
    let profile = CurvedProfile {
        frame: PlaneFrame {
            origin: seat,
            u: mount.frame.u,
            v: back,
        },
        outer: crate::sheet_forming::polygon_edges(&meridian),
        holes: vec![],
    };
    let origins = [
        "head.cap",
        "head.outer",
        "head.shoulder",
        "shaft.outer",
        "shaft.cap",
        "axis",
    ]
    .map(|role| Origin {
        source: feature,
        role: format!("sheet.screw.{role}"),
    });
    Solid::rust_revolve_curved_profile_named(
        &profile,
        RevolutionAxis {
            origin_mm: seat,
            direction: back,
        },
        TAU,
        feature,
        &origins,
        policy,
    )
    .map_err(|e| e.to_string())
}

pub(super) fn append_pieces(
    pieces: &mut Vec<Solid>,
    mount: Mount,
    preset: &ResolvedPreset,
    item: HardwareItem,
    feature: EntityId,
    policy: GeometryPolicy,
) -> Result<(), String> {
    let out = mount.out;
    let back = out.map(|c| -c);
    let geometry = |e: multicad_kernel::GeometryError| e.to_string();
    let piece = match (preset, item) {
        (
            ResolvedPreset::Screw {
                thread,
                length_mm,
                washers,
                ..
            },
            HardwareItem::Screw,
        ) => {
            let d = thread.data();
            let lift = if *washers { d.washer[2] } else { 0. };
            round_head(
                &mount,
                add(mount.surface, out, lift),
                back,
                [d.head[0] / 2., d.head[1], d.diameter / 2., *length_mm],
                feature,
                policy,
            )?
        }
        (ResolvedPreset::Screw { thread, nut, .. }, HardwareItem::Washer) => {
            let [inner, outer, height] = thread.data().washer;
            let ring = |origin, axis| {
                Solid::rust_annulus(
                    inner / 2.,
                    outer / 2.,
                    height,
                    feature,
                    mount.frame_at(origin, axis),
                    policy,
                )
                .map_err(geometry)
            };
            let front = ring(mount.surface, out)?;
            if *nut {
                let rear = ring(mount.rear, back)?;
                pieces.push(front);
                rear
            } else {
                front
            }
        }
        (
            ResolvedPreset::Screw {
                thread, washers, ..
            },
            HardwareItem::Nut,
        ) => {
            let d = thread.data();
            let lift = if *washers { d.washer[2] } else { 0. };
            Solid::rust_cosmetic_hex_nut(
                d.nut[0],
                d.diameter / 2.,
                d.nut[1],
                policy.minimum_feature_mm,
                feature,
                mount.hex_frame(add(mount.rear, back, lift), back),
                policy,
            )
            .map_err(geometry)?
        }
        (
            ResolvedPreset::Standoff {
                thread,
                length_mm,
                across_flats_mm,
            },
            HardwareItem::Standoff,
        ) => Solid::rust_cosmetic_hex_nut(
            *across_flats_mm,
            thread.data().diameter / 2.,
            *length_mm,
            policy.minimum_feature_mm,
            feature,
            mount.hex_frame(mount.surface, out),
            policy,
        )
        .map_err(geometry)?,
        (
            ResolvedPreset::Insert {
                body_mm, height_mm, ..
            },
            HardwareItem::Insert,
        ) => crate::rust_backend::stock_profile(
            feature,
            &CurvedProfile {
                frame: mount.frame_at(mount.surface, out),
                outer: circle(*body_mm),
                holes: vec![],
            },
            &[Origin {
                source: feature,
                role: "sheet.insert.outer".into(),
            }],
            *height_mm,
            policy,
        )
        .map_err(|e| e.message)?,
        _ => return Err("Hardware item does not match its preset".into()),
    };
    pieces.push(piece);
    Ok(())
}
