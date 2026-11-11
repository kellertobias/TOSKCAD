//! Bounded spline surfaces for every start of a straight helical thread.
use super::*;
use crate::{
    headed_screw::{Screw, ScrewHead, hex_vertices},
    helical_thread::Thread,
    step_read::MAX_STEP_ENTITIES,
};

mod rounded;
mod tangent;

pub(crate) const DEGREE: usize = 11;
pub(super) const JETS: usize = DEGREE.div_ceil(2);
const FACTORIAL: f64 = 479_001_600.;
const MAX_ERROR_MM: f64 = 1e-7;

type Complex = [f64; 2];

#[derive(Clone, Copy)]
pub(crate) struct SocketHead {
    pub(crate) radius_mm: f64,
    pub(crate) height_mm: f64,
    pub(crate) across_flats_mm: f64,
    pub(crate) depth_mm: f64,
    pub(crate) shank_length_mm: f64,
}

#[derive(Clone, Copy)]
pub(crate) enum Closure {
    Shaft,
    Bore(f64),
    BlindBore {
        radius_mm: f64,
        stock_length_mm: f64,
    },
    ExternalStock {
        radius_mm: f64,
        stock_length_mm: f64,
    },
    TerminalRelief {
        neck_radius_mm: f64,
        full_length_mm: f64,
    },
    /// External thread with a coaxial through bore of this radius.
    Hollow(f64),
    HexHead {
        across_flats_mm: f64,
        height_mm: f64,
    },
    SocketHead(SocketHead),
    HexNut(f64),
}

impl Closure {
    pub(crate) fn stock(radius: Option<f64>) -> Self {
        radius.map_or(Self::Shaft, Self::Bore)
    }

    pub(crate) fn hollow(radius: Option<f64>) -> Self {
        radius.map_or(Self::Shaft, Self::Hollow)
    }

    /// Radius and outward sense of the closing cylinder wall.
    pub(crate) fn cylinder(self) -> Option<(f64, bool)> {
        match self {
            Self::Bore(radius)
            | Self::BlindBore {
                radius_mm: radius, ..
            } => Some((radius, true)),
            Self::Hollow(radius) => Some((radius, false)),
            _ => None,
        }
    }

    pub(crate) fn screw(screw: &Screw) -> Result<Self, StepError> {
        let spec = screw.spec();
        if spec.shank_length_mm > 0.
            && (!matches!(spec.head, ScrewHead::SocketCap { .. })
                || spec.shank_radius_mm != screw.thread().spec().major_radius_mm)
        {
            return Err(StepError::Unsupported);
        }
        Ok(match spec.head {
            ScrewHead::ExternalHex {
                across_flats_mm,
                height_mm,
            } => Self::HexHead {
                across_flats_mm,
                height_mm,
            },
            ScrewHead::SocketCap {
                radius_mm,
                height_mm,
                socket_across_flats_mm,
                socket_depth_mm,
            } => Self::SocketHead(SocketHead {
                radius_mm,
                height_mm,
                across_flats_mm: socket_across_flats_mm,
                depth_mm: socket_depth_mm,
                shank_length_mm: spec.shank_length_mm,
            }),
        })
    }

    pub(crate) fn is_screw(self) -> bool {
        matches!(self, Self::HexHead { .. } | Self::SocketHead(_))
    }

    pub(crate) fn inside(self) -> bool {
        matches!(
            self,
            Self::Bore(_) | Self::HexNut(_) | Self::BlindBore { .. }
        )
    }

    pub(crate) fn tangent_shank(self) -> Option<f64> {
        if let Self::SocketHead(head) = self
            && head.shank_length_mm > 0.
        {
            Some(head.shank_length_mm)
        } else {
            None
        }
    }

    pub(crate) fn cap_hole(self, top: bool) -> bool {
        (self.inside() && !(top && self.is_blind_bore()))
            || (!top && self.is_screw() && self.tangent_shank().is_none())
    }

    pub(crate) fn is_blind_bore(self) -> bool {
        matches!(self, Self::BlindBore { .. })
    }
    pub(crate) fn cap_positive(self, top: bool) -> bool {
        (top && !self.is_blind_bore()) || self.is_screw()
    }
    pub(crate) fn stock_length(self, thread_length: f64) -> f64 {
        if let Self::BlindBore {
            stock_length_mm, ..
        }
        | Self::ExternalStock {
            stock_length_mm, ..
        }
        | Self::TerminalRelief {
            full_length_mm: stock_length_mm,
            ..
        } = self
        {
            stock_length_mm
        } else {
            thread_length
        }
    }
    pub(crate) fn face_count(self, bands: usize, major: f64) -> usize {
        match self {
            Self::Shaft => bands + 2,
            Self::BlindBore { .. } | Self::TerminalRelief { .. } => bands + 4,
            Self::ExternalStock { radius_mm, .. } => {
                bands + 3 + if radius_mm == major { bands / 4 } else { 1 }
            }
            Self::Bore(_) | Self::Hollow(_) => bands + 3,
            Self::HexHead { .. } => bands + 9,
            Self::HexNut(_) => bands + 8,
            Self::SocketHead(head) => bands + 11 + 2 * usize::from(head.shank_length_mm > 0.),
        }
    }

    fn radial_and_axial_extent(self, major_radius_mm: f64) -> (f64, f64) {
        match self {
            Self::Shaft | Self::Hollow(_) | Self::TerminalRelief { .. } => (major_radius_mm, 0.),
            Self::Bore(radius)
            | Self::BlindBore {
                radius_mm: radius, ..
            }
            | Self::ExternalStock {
                radius_mm: radius, ..
            } => (radius, 0.),
            Self::HexNut(width) => (width / 3_f64.sqrt(), 0.),
            Self::HexHead {
                across_flats_mm,
                height_mm,
            } => (across_flats_mm / 3_f64.sqrt(), height_mm),
            Self::SocketHead(head) => (head.radius_mm, head.height_mm + head.shank_length_mm),
        }
    }
}

pub fn write_thread_end_relief(
    relief: &crate::thread_end_relief::Relief,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_thread_end_relief(relief, origin, u, v, tolerance_mm))
}
pub fn supports_thread_end_relief(
    relief: &crate::thread_end_relief::Relief,
    origin: Point,
    tolerance_mm: f64,
) -> bool {
    prepare(
        relief.material_thread(),
        Closure::TerminalRelief {
            neck_radius_mm: relief.neck_radius_mm(),
            full_length_mm: relief.full_length_mm(),
        },
        origin,
        tolerance_mm,
    )
    .is_ok()
}

pub fn write_threaded_shaft(
    shaft: &crate::threaded_shaft::Shaft,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_threaded_shaft(shaft, origin, u, v, tolerance_mm))
}
pub fn supports_threaded_shaft(
    shaft: &crate::threaded_shaft::Shaft,
    origin: Point,
    tolerance_mm: f64,
) -> bool {
    prepare(
        shaft.thread(),
        Closure::ExternalStock {
            radius_mm: shaft.radius_mm(),
            stock_length_mm: shaft.stock_length_mm(),
        },
        origin,
        tolerance_mm,
    )
    .is_ok()
}

pub fn write_helical_receiver(
    receiver: &crate::helical_receiver::Receiver,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_helical_receiver(receiver, origin, u, v, tolerance_mm))
}
pub fn supports_helical_receiver(
    receiver: &crate::helical_receiver::Receiver,
    origin: Point,
    tolerance_mm: f64,
) -> bool {
    prepare(
        receiver.thread(),
        Closure::BlindBore {
            radius_mm: receiver.radius_mm(),
            stock_length_mm: receiver.stock_length_mm(),
        },
        origin,
        tolerance_mm,
    )
    .is_ok()
}

pub fn write_helical_nut(
    nut: &crate::hex_nut::Nut,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_helical_nut(nut, origin, u, v, tolerance_mm))
}
pub fn supports_helical_nut(nut: &crate::hex_nut::Nut, origin: Point, tolerance_mm: f64) -> bool {
    prepare(
        nut.thread(),
        Closure::HexNut(nut.across_flats_mm()),
        origin,
        tolerance_mm,
    )
    .is_ok()
}

pub fn write_helical_screw(
    screw: &Screw,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| writer.append_helical_screw(screw, origin, u, v, tolerance_mm))
}

/// Whether a fully threaded hex-head or socket-cap screw fits the STEP proof.
pub fn supports_helical_screw(screw: &Screw, origin: Point, tolerance_mm: f64) -> bool {
    Closure::screw(screw)
        .and_then(|closure| prepare(screw.thread(), closure, origin, tolerance_mm))
        .is_ok()
}

pub fn write_helical_thread(
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    origin: Point,
    u: Point,
    v: Point,
    tolerance_mm: f64,
) -> Result<String, StepError> {
    write_one(|writer| {
        writer.append_helical_thread(thread, stock_radius_mm, origin, u, v, tolerance_mm)
    })
}

/// Whether the requested straight thread fits the certified STEP budget.
pub fn supports_helical_thread(
    thread: &Thread,
    stock_radius_mm: Option<f64>,
    origin: Point,
    tolerance_mm: f64,
) -> bool {
    prepare(
        thread,
        Closure::stock(stock_radius_mm),
        origin,
        tolerance_mm,
    )
    .is_ok()
}

pub(super) fn encode(
    file: &mut StepFile,
    thread: &Thread,
    closure: Closure,
    placement: Placement,
    tolerance_mm: f64,
) -> Result<Entity, StepError> {
    let closure_extents = match closure {
        Closure::ExternalStock {
            radius_mm,
            stock_length_mm,
        } => Some((radius_mm, stock_length_mm)),
        Closure::TerminalRelief { full_length_mm, .. } => {
            Some((thread.spec().major_radius_mm, full_length_mm))
        }
        _ => None,
    };
    if let Some((radius_mm, stock_length_mm)) = closure_extents {
        let norm = |[x, y, z]: Point| x.hypot(y).hypot(z);
        let unit = |p: Point| {
            let n = norm(p);
            p.map(|v| v / n)
        };
        let normal = unit(placement.normal);
        let v = unit(cross(normal, unit(placement.u)));
        let u = cross(v, normal);
        // STEP directions are normalized by its frame semantics. Bound the
        // complete retained stock, not only the much smaller thread nets.
        // Preparation separately reserves 128 eps * scale for arithmetic.
        let residual = norm(std::array::from_fn(|i| placement.u[i] - u[i])) * radius_mm
            + norm(std::array::from_fn(|i| placement.v[i] - v[i])) * radius_mm
            + norm(std::array::from_fn(|i| placement.normal[i] - normal[i])) * stock_length_mm;
        if !residual.is_finite() || residual > tolerance_mm.min(MAX_ERROR_MM) / 8. {
            return Err(StepError::InvalidPlacement);
        }
    }
    let ThreadSplines {
        bands,
        axial,
        heights,
    } = prepare(thread, closure, placement.origin, tolerance_mm)?;
    let spec = thread.spec();
    let height = spec.length_mm;
    let [axial_start, axial_end] = [0, axial.poles.len() - 1];
    let endpoints = [axial.poles[axial_start], axial.poles[axial_end]];
    let rings = [0., height]
        .into_iter()
        .zip(endpoints)
        .map(|(z, spin)| {
            bands
                .iter()
                .map(|band| file.vertex(placement.point_f64(product(band.poles[0], spin), z)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let rails = bands
        .iter()
        .enumerate()
        .map(|(i, band)| {
            let [bottom, top] = [rings[0][i], rings[1][i]];
            let mut poles = axial
                .poles
                .iter()
                .zip(&heights)
                .map(|(&spin, &z)| placement.point_f64(product(band.poles[0], spin), z));
            curve(file, &axial, [bottom, top], &mut poles)
        })
        .collect::<Vec<_>>();
    let external = if let Closure::ExternalStock {
        radius_mm,
        stock_length_mm,
    } = closure
    {
        if radius_mm == spec.major_radius_mm {
            let trim = tangent::external(
                file,
                &placement,
                thread,
                stock_length_mm,
                (0..spec.starts as usize)
                    .map(|k| [rings[1][4 * k], rings[1][4 * k + 3]])
                    .collect(),
            );
            Some(trim?)
        } else {
            None
        }
    } else {
        None
    };
    let shank = closure
        .tangent_shank()
        .map(|length| tangent::encode(file, &placement, thread, length, [rings[0][0], rings[0][3]]))
        .transpose()?;
    let mut faces = Vec::with_capacity(closure.face_count(bands.len(), spec.major_radius_mm));
    let mut cap_edges = [
        Vec::with_capacity(bands.len()),
        Vec::with_capacity(bands.len()),
    ];
    let forward = spec.left_hand != closure.inside();
    for (i, band) in bands.iter().enumerate() {
        let next = (i + 1) % bands.len();
        let rims = [0, 1].map(|end| {
            if end == 1
                && i % 4 == 3
                && let Some(external) = &external
            {
                return external.crests[i / 4];
            }
            if end == 0
                && i == 3
                && let Some(shank) = &shank
            {
                return shank.crest;
            }
            let z = [0., height][end];
            let mut poles = band
                .poles
                .iter()
                .map(|&radial| placement.point_f64(product(radial, endpoints[end]), z));
            curve(file, band, [rings[end][i], rings[end][next]], &mut poles)
        });
        for (cap, rim) in cap_edges.iter_mut().zip(rims) {
            cap.push(rim);
        }
        let surface = surface(file, band, &axial, &heights, &placement);
        let uses = [
            (rims[0], true),
            (rails[next], true),
            (rims[1], false),
            (rails[i], false),
        ];
        let edges = if forward {
            uses.map(|(edge, sense)| file.oriented_edge(edge, sense))
        } else {
            [3, 2, 1, 0].map(|i| file.oriented_edge(uses[i].0, !uses[i].1))
        };
        let bound = file.bound(&edges, true);
        faces.push(file.face_on(&[bound], surface, forward));
    }

    let centers = [0., height].map(|z| placement.point_f64([0.; 2], z));
    let cylinder_spec = if let Closure::ExternalStock {
        radius_mm,
        stock_length_mm,
    } = closure
    {
        if external.is_none() {
            Some((radius_mm, true, [height, stock_length_mm]))
        } else {
            None
        }
    } else {
        if let Closure::TerminalRelief {
            neck_radius_mm,
            full_length_mm,
        } = closure
        {
            Some((neck_radius_mm, true, [height, full_length_mm]))
        } else {
            closure
                .cylinder()
                .map(|(radius, outward)| (radius, outward, [0., closure.stock_length(height)]))
        }
    };
    let cylinder = if let Some((radius, outward, ends)) = cylinder_spec {
        let stock_centers = ends.map(|z| placement.point_f64([0.; 2], z));
        let trim = CylindricalTrim::new(file, stock_centers, placement.normal, placement.u, radius);
        Some((trim?, outward))
    } else {
        None
    };
    let head = match closure {
        Closure::HexHead {
            across_flats_mm,
            height_mm,
        } => {
            let trim = encode_hex_head(file, &placement, across_flats_mm, height_mm);
            Some(trim?)
        }
        Closure::SocketHead(socket) => Some(encode_socket_head(file, &placement, socket)?),
        Closure::Shaft
        | Closure::Bore(_)
        | Closure::BlindBore { .. }
        | Closure::ExternalStock { .. }
        | Closure::TerminalRelief { .. }
        | Closure::Hollow(_)
        | Closure::HexNut(_) => None,
    };
    let hex = if let Closure::HexNut(width) = closure {
        Some(encode_hex_prism(
            file,
            &placement,
            width,
            [0., height],
            false,
        )?)
    } else {
        None
    };
    let mut extra_shoulders = Vec::new();
    for (end, cap) in cap_edges.into_iter().enumerate() {
        let top = end == 1;
        if top && let Some(external) = &external {
            for (k, part) in cap.as_chunks::<4>().0.iter().enumerate() {
                let uses = [
                    (part[0], true),
                    (part[1], true),
                    (part[2], true),
                    (external.complements[k], false),
                ];
                let wire = if spec.left_hand {
                    uses.map(|(e, s)| file.oriented_edge(e, s))
                } else {
                    [3, 2, 1, 0].map(|i| file.oriented_edge(uses[i].0, !uses[i].1))
                };
                let bound = file.bound(&wire, true);
                let center = file.point(centers[1]);
                let face = file.face(&[bound], center, placement.normal.map(|n| -n), placement.u);
                if k == 0 {
                    faces.push(face);
                } else {
                    extra_shoulders.push(face);
                }
            }
            continue;
        }
        let ring_forward = top == forward;
        let mut uses = cap.into_iter().map(|edge| (edge, true)).collect::<Vec<_>>();
        if !top && let Some(shank) = &shank {
            uses[3] = (shank.complement, false);
        }
        let edges = if ring_forward {
            uses.into_iter()
                .map(|(edge, sense)| file.oriented_edge(edge, sense))
                .collect::<Vec<_>>()
        } else {
            uses.into_iter()
                .rev()
                .map(|(edge, sense)| file.oriented_edge(edge, !sense))
                .collect::<Vec<_>>()
        };
        let outside = matches!(closure, Closure::ExternalStock { .. });
        let relief = matches!(closure, Closure::TerminalRelief { .. });
        let trim = file.bound(&edges, !closure.cap_hole(top) && !(outside && top));
        let mut bounds = Vec::with_capacity(2);
        if let Some((cylinder, outward)) = &cylinder
            && !(top && closure.is_blind_bore())
            && !((outside || relief) && !top)
        {
            if relief {
                bounds.push(circular_bound(file, cylinder.rims[0], false, false));
            } else {
                bounds.push(cylinder.cap_bound(file, top && !outside, *outward));
            }
        }
        if let Some((rims, _)) = &hex {
            bounds.push(rims[end]);
        }
        if !top
            && shank.is_none()
            && let Some((shoulder, _)) = &head
        {
            bounds.push(*shoulder);
        }
        bounds.push(trim);
        let center = file.point(centers[end]);
        let outward = closure.cap_positive(top) && !outside;
        let normal = placement
            .normal
            .map(|value| if outward { value } else { -value });
        faces.push(file.face(&bounds, center, normal, placement.u));
    }
    faces.extend(extra_shoulders);
    if let Some((cylinder, outward)) = &cylinder {
        faces.push(cylinder.wall(file, *outward));
        if closure.is_blind_bore()
            || matches!(
                closure,
                Closure::ExternalStock { .. } | Closure::TerminalRelief { .. }
            )
        {
            let bound = cylinder.cap_bound(file, true, true);
            let center = file.point(placement.point_f64([0.; 2], closure.stock_length(height)));
            faces.push(file.face(&[bound], center, placement.normal, placement.u));
        }
    }
    if let Some(external) = external {
        faces.push(external.wall);
        faces.push(external.far_cap);
    }
    if let Some((_, walls)) = hex {
        faces.extend(walls);
    }
    if let Some((head_seat, head_faces)) = head {
        if let Some(shank) = shank {
            let length = closure.tangent_shank().expect("existing shank");
            let seat = file.point(placement.point_f64([0.; 2], -length));
            faces.push(shank.wall);
            faces.push(file.face(
                &[head_seat, shank.seat_hole],
                seat,
                placement.normal,
                placement.u,
            ));
        }
        faces.extend(head_faces);
    }
    Ok(file.solid(&faces))
}

fn encode_hex_head(
    file: &mut StepFile,
    placement: &Placement,
    across_flats_mm: f64,
    height_mm: f64,
) -> Result<(Entity, Vec<Entity>), StepError> {
    let ([bottom, shoulder], mut faces) =
        encode_hex_prism(file, placement, across_flats_mm, [-height_mm, 0.], false)?;
    let center = file.point(placement.point_f64([0.; 2], -height_mm));
    let down = placement.normal.map(|n| -n);
    faces.insert(0, file.face(&[bottom], center, down, placement.u));
    Ok((shoulder, faces))
}

fn encode_socket_head(
    file: &mut StepFile,
    placement: &Placement,
    head: SocketHead,
) -> Result<(Entity, Vec<Entity>), StepError> {
    let seat = -head.shank_length_mm;
    let lower = seat - head.height_mm;
    let floor = lower + head.depth_mm;
    let center = |z| placement.point_f64([0.; 2], z);
    let shell = CylindricalTrim::new(
        file,
        [center(lower), center(seat)],
        placement.normal,
        placement.u,
        head.radius_mm,
    )?;
    let ([socket, floor_bound], sides) =
        encode_hex_prism(file, placement, head.across_flats_mm, [lower, floor], true)?;
    let shoulder = shell.cap_bound(file, true, true);
    let bottom = shell.cap_bound(file, false, true);
    let down = placement.normal.map(|n| -n);
    let end_face = file.face(&[bottom, socket], shell.centers[0], down, placement.u);
    let wall = shell.wall(file, true);
    let floor_center = file.point(center(floor));
    let floor = file.face(&[floor_bound], floor_center, down, placement.u);
    let mut faces = vec![end_face, wall, floor];
    faces.extend(sides);
    Ok((shoulder, faces))
}

fn encode_hex_prism(
    file: &mut StepFile,
    placement: &Placement,
    across_flats_mm: f64,
    depths: [f64; 2],
    cavity: bool,
) -> Result<([Entity; 2], Vec<Entity>), StepError> {
    let hex = hex_vertices(across_flats_mm);
    let mut prism = BRep::new(
        file,
        depths
            .into_iter()
            .flat_map(|z| hex.map(|point| placement.point_f64(point, z)))
            .collect(),
    );
    let rims = if cavity {
        [
            prism.bound(&[0, 1, 2, 3, 4, 5], false)?,
            prism.bound(&[11, 10, 9, 8, 7, 6], true)?,
        ]
    } else {
        [
            prism.bound(&[5, 4, 3, 2, 1, 0], true)?,
            prism.bound(&[6, 7, 8, 9, 10, 11], true)?,
        ]
    };
    let mut faces = Vec::with_capacity(6);
    for i in 0..6 {
        let next = (i + 1) % 6;
        let indices = if cavity {
            [i, i + 6, next + 6, next]
        } else {
            [i, next, next + 6, i + 6]
        };
        let bound = prism.bound(&indices, true)?;
        let from = prism.vertices[i];
        let u = direction(from.point, prism.vertices[next].point)?;
        let normal = cross(u, placement.normal).map(|n| if cavity { -n } else { n });
        faces.push(prism.file.face(&[bound], from.cartesian, normal, u));
    }
    Ok((rims, faces))
}

pub(crate) struct ThreadSplines {
    pub(crate) bands: Vec<Spline>,
    pub(crate) axial: Spline,
    pub(crate) heights: Vec<f64>,
}

pub(crate) fn prepare(
    thread: &Thread,
    closure: Closure,
    origin: Point,
    tolerance_mm: f64,
) -> Result<ThreadSplines, StepError> {
    let spec = thread.spec();
    if spec.starts != 1
        && !matches!(
            closure,
            Closure::Shaft
                | Closure::Bore(_)
                | Closure::HexNut(_)
                | Closure::BlindBore { .. }
                | Closure::ExternalStock { .. }
                | Closure::TerminalRelief { .. }
        )
    {
        return Err(StepError::Unsupported);
    }
    if !tolerance_mm.is_finite()
        || tolerance_mm <= 0.
        || matches!(closure, Closure::Bore(r) if !r.is_finite() || r <= spec.major_radius_mm)
        || matches!(closure, Closure::BlindBore { radius_mm,stock_length_mm } if crate::helical_receiver::Receiver::new(*thread,radius_mm,stock_length_mm).is_err())
        || matches!(closure, Closure::ExternalStock {radius_mm,stock_length_mm} if crate::threaded_shaft::Shaft::new(*thread,radius_mm,stock_length_mm).is_err())
        || matches!(closure,Closure::TerminalRelief {neck_radius_mm,full_length_mm} if !neck_radius_mm.is_finite() || neck_radius_mm<=0. || neck_radius_mm>=thread.root_radius_mm() || !full_length_mm.is_finite() || full_length_mm<=spec.length_mm || full_length_mm>1e7)
        || matches!(closure, Closure::HexNut(width) if !width.is_finite() || width / 2. <= spec.major_radius_mm)
        || matches!(closure, Closure::Hollow(r)
            if !r.is_finite() || r <= 0. || r >= spec.major_radius_mm - spec.depth_mm)
    {
        return Err(StepError::InvalidGeometry);
    }
    let tolerance = tolerance_mm.min(MAX_ERROR_MM);
    let height = spec.length_mm;
    let major = spec.major_radius_mm;
    let rate = thread.helical_rate();
    let spin = height / rate.abs();
    let (outer_radius, below) = closure.radial_and_axial_extent(major);
    let scale = origin.into_iter().map(f64::abs).fold(0., f64::max)
        + outer_radius
        + below
        + major * spin
        + closure.stock_length(height)
        + 1.;
    // Reserve room for floating-point and decimal STEP representation after
    // rotating/translating the polynomial poles.
    if 128. * f64::EPSILON * scale > tolerance / 2.
        || spec.major_radius_mm - spec.depth_mm <= tolerance
        || matches!(closure, Closure::Bore(radius) if radius - major <= tolerance)
        || matches!(closure, Closure::BlindBore { radius_mm,stock_length_mm } if radius_mm-major<=tolerance || stock_length_mm-height<=tolerance)
        || matches!(closure, Closure::ExternalStock {radius_mm,stock_length_mm} if (radius_mm!=major && radius_mm-major<=tolerance) || stock_length_mm-height<=tolerance)
        || matches!(closure,Closure::TerminalRelief {neck_radius_mm,full_length_mm} if neck_radius_mm<=tolerance || thread.root_radius_mm()-neck_radius_mm<=tolerance || full_length_mm-height<=tolerance)
        || matches!(closure, Closure::HexNut(width) if width / 2. - major <= tolerance)
        || matches!(closure, Closure::Hollow(radius)
            if radius <= tolerance || major - spec.depth_mm - radius <= tolerance)
        || matches!(closure, Closure::HexHead { across_flats_mm, height_mm }
            if across_flats_mm / 2. - major <= tolerance || height_mm <= tolerance)
        || matches!(closure, Closure::SocketHead(head) if head.radius_mm - major <= tolerance
            || head.radius_mm - head.across_flats_mm / 3_f64.sqrt() <= tolerance
            || head.depth_mm <= tolerance || head.height_mm - head.depth_mm <= tolerance
            || (head.shank_length_mm > 0. && head.shank_length_mm <= tolerance))
    {
        return Err(StepError::Limit);
    }

    // Use dimensionless angular coordinates and a separable tensor product.
    // XY(p,z) = A(p) B(z), where A carries the radius and B is unit rotation.
    // Bound the factors separately. The product error is at most
    // e_A + R e_B + e_A e_B, leaving room for roundoff and the closure seam.
    let hand = rate.signum();
    let axial = Spline::new(0., spin, [1., 1.], hand, tolerance / (8. * major))?;
    let knots = thread.profile_breaks();
    let [_, start, upper, lower, crest, _] = knots;
    let phase = [start, upper, lower, crest, start + spec.pitch_mm];
    let circle = thread.root_circle();
    let root_radii = circle.map_or_else(
        || [thread.root_radius_mm(); 2],
        |_| [upper, lower].map(|phase| thread.radius_at_phase(phase)),
    );
    let radii = [major, root_radii[0], root_radii[1], major, major];
    let mut bands = Vec::with_capacity(4 * spec.starts as usize);
    for start_index in 0..spec.starts {
        let shift = f64::from(start_index) * spec.pitch_mm;
        let shifted = phase.map(|phase| phase + shift);
        for (band, (p, r)) in shifted.windows(2).zip(radii.windows(2)).enumerate() {
            let spline = if band == 1
                && let Some(mut circle) = circle
            {
                circle.phase += shift;
                Spline::rounded_root(
                    circle,
                    [p[0], p[1]],
                    [r[0], r[1]],
                    rate.abs(),
                    -hand,
                    tolerance / 8.,
                )?
            } else {
                Spline::new(
                    p[0] / rate.abs(),
                    p[1] / rate.abs(),
                    [r[0], r[1]],
                    -hand,
                    tolerance / 8.,
                )?
            };
            bands.push(spline);
        }
    }
    // Every start is part of one continuous transverse material boundary.
    // Reuse actual endpoint poles at all joins, not merely the global wrap,
    // so independent evaluation rounding cannot create unmatched seam wires.
    for i in 1..bands.len() {
        bands[i].poles[0] = *bands[i - 1].poles.last().expect("nonempty spline");
    }
    let seam = bands[0].poles[0];
    *bands
        .last_mut()
        .expect("analytic bands")
        .poles
        .last_mut()
        .expect("nonempty spline") = seam;
    if bands.iter().any(|band| !band.advances_angle(-hand)) {
        return Err(StepError::Limit);
    }
    if closure.tangent_shank().is_some()
        || matches!(closure,Closure::ExternalStock {radius_mm,..} if radius_mm==major)
    {
        for part in bands.as_chunks::<4>().0 {
            check_shank_clearance(&part[..3], major)?;
        }
    }
    let poles = bands.iter().map(|band| band.poles.len()).sum::<usize>();
    if poles > (MAX_STEP_ENTITIES / 2) / axial.poles.len() {
        return Err(StepError::Limit);
    }

    let heights = axial.linear_poles(height);
    Ok(ThreadSplines {
        bands,
        axial,
        heights,
    })
}

pub(crate) struct Spline {
    pub(crate) knots: Vec<f64>,
    pub(crate) poles: Vec<Complex>,
}

fn check_shank_clearance(inward: &[Spline], radius: f64) -> Result<(), StepError> {
    if inward.iter().any(|band| !band.below_radius(radius)) {
        return Err(StepError::Limit);
    }
    Ok(())
}

impl Spline {
    pub(super) fn new(
        a: f64,
        b: f64,
        radii: [f64; 2],
        sense: f64,
        error: f64,
    ) -> Result<Self, StepError> {
        let span = b - a;
        let slope = (radii[1] - radii[0]) / span;
        let max_derivative = radii[0].max(radii[1]) + (DEGREE + 1) as f64 * slope.abs();
        // The two-end Hermite remainder is f^(12)(ξ)(t-a)^6(t-b)^6 / 12!.
        // Applying the scalar bound to a projection along the error vector
        // bounds the complex norm without a component-wise penalty.
        let span_limit = 2. * (error * FACTORIAL / max_derivative).powf(1. / 12.);
        Self::with_jets(a, b, span_limit, |t| {
            let r = if t == a {
                radii[0]
            } else if t == b {
                radii[1]
            } else {
                radii[0] + slope * (t - a)
            };
            affine_jet(t, r, slope, sense)
        })
    }

    pub(super) fn with_jets(
        a: f64,
        b: f64,
        span_limit: f64,
        jet: impl Fn(f64) -> [Complex; JETS],
    ) -> Result<Self, StepError> {
        let span = b - a;
        let segments = (span / span_limit).ceil();
        if !(1. ..=1024.).contains(&segments) {
            return Err(StepError::Limit);
        }
        let segments = segments as usize;
        let knots = (0..=segments)
            .map(|i| {
                if i == segments {
                    b
                } else {
                    a + span * i as f64 / segments as f64
                }
            })
            .collect::<Vec<_>>();
        let mut poles = Vec::with_capacity(segments * DEGREE + 1);
        for pair in knots.windows(2) {
            let [start, end] = [pair[0], pair[1]];
            let length = end - start;
            if poles.is_empty() {
                poles.extend(control_half(jet(start), length));
            } else {
                poles.extend(control_half(jet(start), length).into_iter().skip(1));
            }
            poles.extend(control_half(jet(end), -length).into_iter().rev());
        }
        Ok(Self { knots, poles })
    }

    pub(super) fn linear_poles(&self, height: f64) -> Vec<f64> {
        // Bézier controls of an affine axial coordinate. This keeps caps
        // exactly planar and does not approximate the lead.
        let end = self.knots[self.knots.len() - 1];
        let mut result = vec![0.];
        for pair in self.knots.windows(2) {
            result.extend((1..=DEGREE).map(|i| {
                let z = if i == DEGREE {
                    pair[1]
                } else {
                    pair[0] + (pair[1] - pair[0]) * i as f64 / DEGREE as f64
                };
                z / end * height
            }));
        }
        result
    }

    pub(super) fn advances_angle(&self, direction: f64) -> bool {
        self.poles.windows(DEGREE + 1).step_by(DEGREE).all(|poles| {
            // The numerator of d(arg(P))/dt is cross(P,P'). A Bernstein
            // polynomial stays in the hull of its coefficients. Multiplying
            // both factors gives a degree-21 Bernstein polynomial, whose
            // sign must agree throughout this span with the exact helix.
            (0..2 * DEGREE).all(|degree| {
                let first = degree.saturating_sub(DEGREE - 1);
                let last = degree.min(DEGREE);
                let coefficient = (first..=last)
                    .map(|i| {
                        let j = degree - i;
                        let a = poles[i];
                        let b = std::array::from_fn::<_, 2, _>(|k| poles[j + 1][k] - poles[j][k]);
                        let weight = binomial(DEGREE, i) * binomial(DEGREE - 1, j);
                        weight * (a[0] * b[1] - a[1] * b[0])
                    })
                    .fold((0., 0.), |(sum, absolute), term| {
                        (sum + term, absolute + term.abs())
                    });
                direction * coefficient.0 > 64. * f64::EPSILON * coefficient.1.max(1.)
            })
        })
    }

    fn below_radius(&self, radius: f64) -> bool {
        let radius_squared = radius * radius;
        self.poles.windows(DEGREE + 1).step_by(DEGREE).all(|poles| {
            // A Bernstein bound on the squared radial distance of each
            // inward chart. It may meet the shank at the two shared ends.
            (0..=2 * DEGREE).all(|degree| {
                let coefficient = (degree.saturating_sub(DEGREE)..=degree.min(DEGREE))
                    .map(|i| {
                        let j = degree - i;
                        let a = poles[i];
                        let b = poles[j];
                        binomial(DEGREE, i) * binomial(DEGREE, j) * (a[0] * b[0] + a[1] * b[1])
                    })
                    .sum::<f64>()
                    / binomial(2 * DEGREE, degree);
                coefficient <= radius_squared * (1. + 128. * f64::EPSILON)
            })
        })
    }

    fn multiplicities(&self) -> String {
        let mut values = vec![DEGREE.to_string(); self.knots.len()];
        values[0] = (DEGREE + 1).to_string();
        values[self.knots.len() - 1] = values[0].clone();
        values.join(",")
    }

    fn knot_values(&self) -> String {
        self.knots
            .iter()
            .map(|knot| format!("{knot:.16E}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn binomial(n: usize, k: usize) -> f64 {
    (1..=k.min(n - k)).fold(1., |value, i| value * (n + 1 - i) as f64 / i as f64)
}

pub(crate) fn product([ax, ay]: Complex, [bx, by]: Complex) -> Complex {
    [ax * bx - ay * by, ax * by + ay * bx]
}

fn affine_jet(t: f64, radius: f64, slope: f64, sense: f64) -> [Complex; JETS] {
    let (sin, cos) = (sense * t).sin_cos();
    let rotation = [cos, sin];
    let imaginary = [0., sense];
    let mut power = [1., 0.];
    let mut jet = [[0.; 2]; JETS];
    for (k, derivative) in jet.iter_mut().enumerate() {
        let radial = [radius, -sense * k as f64 * slope];
        *derivative = product(product(rotation, power), radial);
        power = product(power, imaginary);
    }
    jet
}

fn control_half(jet: [Complex; JETS], length: f64) -> [Complex; JETS] {
    std::array::from_fn(|i| {
        let mut point = jet[0];
        let mut weight = 1.;
        for (k, derivative) in jet.iter().enumerate().take(i + 1).skip(1) {
            weight *= length * (i - k + 1) as f64 / ((DEGREE - k + 1) * k) as f64;
            point[0] += derivative[0] * weight;
            point[1] += derivative[1] * weight;
        }
        point
    })
}

fn curve(
    file: &mut StepFile,
    spline: &Spline,
    [from, to]: [Vertex; 2],
    points: &mut dyn Iterator<Item = Point>,
) -> Entity {
    let poles = points
        .enumerate()
        .map(|(i, point)| {
            if i == 0 {
                from.cartesian
            } else if i + 1 == spline.poles.len() {
                to.cartesian
            } else {
                file.point(point)
            }
        })
        .collect::<Vec<_>>();
    let multiplicities = spline.multiplicities();
    let knots = spline.knot_values();
    let curve = file.entity(format_args!(
        "B_SPLINE_CURVE_WITH_KNOTS('',{DEGREE},{},.UNSPECIFIED.,.F.,.F.,({multiplicities}),({knots}),.UNSPECIFIED.)",
        Entities(&poles)
    ));
    let (start, end) = (from.vertex, to.vertex);
    file.entity(format_args!("EDGE_CURVE('',{start},{end},{curve},.T.)"))
}

fn surface(
    file: &mut StepFile,
    radial: &Spline,
    axial: &Spline,
    heights: &[f64],
    placement: &Placement,
) -> Entity {
    let rows = radial
        .poles
        .iter()
        .map(|&radial| {
            axial
                .poles
                .iter()
                .zip(heights)
                .map(|(&spin, &z)| file.point(placement.point_f64(product(radial, spin), z)))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let net = rows
        .iter()
        .map(|row| Entities(row).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let u_mult = radial.multiplicities();
    let v_mult = axial.multiplicities();
    let u_knots = radial.knot_values();
    let v_knots = axial.knot_values();
    file.entity(format_args!(
        "B_SPLINE_SURFACE_WITH_KNOTS('',{DEGREE},{DEGREE},({net}),.UNSPECIFIED.,.F.,.F.,.F.,({u_mult}),({v_mult}),({u_knots}),({v_knots}),.UNSPECIFIED.)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helical_thread::Spec;

    fn thread(starts: u32, left_hand: bool) -> Thread {
        Thread::new(Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 3.7,
            depth_mm: 5. * 3_f64.sqrt() / 16.,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts,
            left_hand,
            root_radius_mm: None,
        })
        .unwrap()
    }

    #[test]
    fn socket_encoder_rejects_a_degenerate_outer_wall() {
        let mut file = StepFile::new();
        let placement = Placement::new([0.; 3], [1., 0., 0.], [0., 1., 0.]).unwrap();
        let invalid = SocketHead {
            radius_mm: 0.,
            height_mm: 6.,
            across_flats_mm: 5.,
            depth_mm: 3.,
            shank_length_mm: 0.,
        };
        assert_eq!(
            encode_socket_head(&mut file, &placement, invalid).err(),
            Some(StepError::InvalidGeometry)
        );
    }

    #[test]
    fn external_encoder_rejects_unrepresentable_tangent_and_collapsed_cylinder_charts() {
        let source = thread(2, false);
        let big = Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 1e4,
            ..source.spec()
        })
        .unwrap();
        let mut file = StepFile::new();
        let placement = Placement::new([0.; 3], [1. + 4e-11, 0., 0.], [0., 1., 0.]).unwrap();
        assert!(
            encode(
                &mut file,
                &big,
                Closure::ExternalStock {
                    radius_mm: 1e4,
                    stock_length_mm: 5.
                },
                placement,
                1e-7
            )
            .is_err()
        );
        let placement = Placement {
            origin: [0.; 3],
            u: [0.; 3],
            v: [0.; 3],
            normal: [0.; 3],
        };
        assert!(
            encode(
                &mut file,
                &source,
                Closure::ExternalStock {
                    radius_mm: 4.,
                    stock_length_mm: 5.
                },
                placement,
                1e-7
            )
            .is_err()
        );
    }

    #[test]
    fn refuses_a_spline_that_crosses_the_tangent_shank_radius() {
        let band = Spline::new(0., 1., [2.9, 3.01], 1., 1e-8).unwrap();
        assert_eq!(check_shank_clearance(&[band], 3.), Err(StepError::Limit));
    }

    #[test]
    fn bounded_spline_matches_rotation_and_affine_radius_between_knots() {
        for sense in [-1., 1.] {
            for radii in [[1., 1.], [3., 2.4], [2.4, 3.]] {
                let error = 1e-8;
                let [a, b] = [0., 7.4];
                let spline = Spline::new(a, b, radii, sense, error).unwrap();
                let linear = spline.linear_poles(5.);
                for (i, pair) in spline.knots.windows(2).enumerate() {
                    for j in 0..=17 {
                        let t = j as f64 / 17.;
                        let x = pair[0] + t * (pair[1] - pair[0]);
                        let r = radii[0] + (radii[1] - radii[0]) * (x - a) / (b - a);
                        let (sin, cos) = (sense * x).sin_cos();
                        let idx = i * DEGREE;
                        let mut controls: [[f64; 3]; DEGREE + 1] = std::array::from_fn(|j| {
                            let [a, b] = spline.poles[idx + j];
                            [a, b, linear[idx + j]]
                        });
                        for n in (1..=DEGREE).rev() {
                            for k in 0..n {
                                controls[k] = std::array::from_fn(|l| {
                                    (1. - t) * controls[k][l] + t * controls[k + 1][l]
                                });
                            }
                        }
                        assert!((controls[0][0] - r * cos).hypot(controls[0][1] - r * sin) < error);
                        assert!((controls[0][2] - x / b * 5.).abs() < 1e-13);
                    }
                }
            }
        }
    }

    #[test]
    fn threaded_exchange_bounds_its_domain_and_retains_both_handings() {
        let origin = [0.; 3];
        let u = [1., 0., 0.];
        let v = [0., 1., 0.];
        let write = |thread: &Thread, radius, tolerance| {
            write_helical_thread(thread, radius, origin, u, v, tolerance)
        };
        for (left, radius) in [
            (false, None),
            (true, Some(4.5)),
            (true, None),
            (false, Some(4.5)),
        ] {
            let step = write(&thread(1, left), radius, 1e-7).unwrap();
            assert_eq!(step.matches("B_SPLINE_SURFACE_WITH_KNOTS").count(), 4);
            assert_eq!(
                step.matches("CYLINDRICAL_SURFACE").count(),
                usize::from(radius.is_some())
            );
            assert_eq!(step.matches("MANIFOLD_SOLID_BREP").count(), 1);
            assert_eq!(
                step.matches("ADVANCED_FACE").count(),
                6 + usize::from(radius.is_some())
            );
            assert!(!step.contains("FACETED_BREP"));
            let verify = |input: &str| {
                crate::step_read::verify_helical_thread(
                    input,
                    &thread(1, left),
                    radius,
                    origin,
                    u,
                    v,
                    1e-7,
                )
            };
            let volume = verify(&step).unwrap();
            let source_volume = thread(1, left).volume_mm3();
            let expected = radius.map_or(source_volume, |radius| {
                std::f64::consts::PI * radius * radius * 3.7 - source_volume
            });
            assert!((volume - expected).abs() < 1e-6);
            assert_ne!(volume, expected);

            let changed_degree = step.replacen(
                "B_SPLINE_SURFACE_WITH_KNOTS('',11,11",
                "B_SPLINE_SURFACE_WITH_KNOTS('',11,10",
                1,
            );
            let changed_units = step.replacen(".MILLI.,.METRE.", "$,.METRE.", 1);
            let changed_control =
                step.replacen("CARTESIAN_POINT('',(", "CARTESIAN_POINT('',(1000.,", 1);
            for changed in [changed_degree, changed_units, changed_control] {
                assert_ne!(changed, step);
                assert!(verify(&changed).is_err());
            }
        }
        let multiple = thread(2, false);
        let multiple_step = write(&multiple, None, 1e-7).unwrap();
        assert!(crate::step_read::read_import_solid(&multiple_step, 1e-7).is_ok());
        assert!(matches!(
            prepare(&multiple, Closure::Hollow(1.), origin, 1e-7),
            Err(StepError::Unsupported)
        ));
        assert_eq!(
            write(&thread(1, false), Some(3.), 1e-7),
            Err(StepError::InvalidGeometry)
        );
        assert_eq!(
            write(&thread(1, false), None, f64::NAN),
            Err(StepError::InvalidGeometry)
        );
        assert_eq!(write(&thread(1, false), None, 1e-16), Err(StepError::Limit));
        assert!(matches!(
            Spline::new(0., 1e12, [1., 1.], 1., 1e-8),
            Err(StepError::Limit)
        ));
        assert!(supports_helical_thread(
            &thread(1, false),
            None,
            origin,
            1e-7
        ));
        let narrow_crest = Thread::new(Spec {
            major_radius_mm: 0.001,
            pitch_mm: 0.001,
            length_mm: 0.0037,
            depth_mm: 0.0005,
            crest_width_mm: 1e-12,
            ..thread(1, false).spec()
        })
        .unwrap();
        assert_eq!(write(&narrow_crest, None, 1e-7), Err(StepError::Limit));
        let long = Thread::new(Spec {
            length_mm: 124.,
            ..thread(1, false).spec()
        })
        .unwrap();
        let encoded = write(&long, None, 1e-7).unwrap();
        let volume =
            crate::step_read::verify_helical_thread(&encoded, &long, None, origin, u, v, 1e-7)
                .unwrap();
        assert!((volume - long.volume_mm3()).abs() < 2e-4);
        let wide = Thread::new(Spec {
            major_radius_mm: 2000.,
            pitch_mm: 1000.,
            length_mm: 124_000.,
            depth_mm: 500.,
            crest_width_mm: 125.,
            lower_flank_deg: 1.,
            upper_flank_deg: 1.,
            ..long.spec()
        })
        .unwrap();
        assert_eq!(write(&wide, None, 1e-7).err(), Some(StepError::Limit));
    }
    #[test]
    fn unresolved_shifted_flank_or_rounded_root_never_collapses_into_a_false_thread() {
        let tiny = Thread::new(Spec {
            major_radius_mm: 1.,
            pitch_mm: 1.,
            length_mm: 1.,
            depth_mm: 2e-16,
            crest_width_mm: 4e-16,
            lower_flank_deg: 80.,
            upper_flank_deg: 1.,
            starts: 2,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        assert!(matches!(
            prepare(&tiny, Closure::Shaft, [0.; 3], 1e-7),
            Err(StepError::Limit)
        ));
        // The original root has distinct valid tangencies. Adding the next
        // start's pitch loses enough phase precision that its lower endpoint
        // lies beyond the circle chart; exporting must refuse that geometry.
        let rounded = Thread::new(Spec {
            major_radius_mm: 1.1538127035505954,
            pitch_mm: 1.,
            length_mm: 1.,
            depth_mm: 0.15381270355059526,
            crest_width_mm: 0.125,
            lower_flank_deg: 1.,
            upper_flank_deg: 80.,
            starts: 2,
            left_hand: false,
            root_radius_mm: Some(8.002582779285636e-16),
        })
        .unwrap();
        assert!(matches!(
            prepare(&rounded, Closure::Shaft, [0.; 3], 1e-7),
            Err(StepError::Limit)
        ));
    }
}

#[cfg(test)]
mod nut_tests;
