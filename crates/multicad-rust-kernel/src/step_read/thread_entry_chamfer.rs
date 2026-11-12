//! Complete finite source and parameter-curve proof for conical thread entries.
use super::*;
use crate::thread_entry_chamfer::{Chamfer, trim::I};
use helical::{face, require, ring};

mod cone;
mod frame;
mod graph;
mod known;
mod polynomial;
mod surface;
mod tensor;

struct Work<'a> {
    remaining: usize,
    interrupt: Option<&'a dyn Fn() -> bool>,
    cancelled: bool,
}
impl Work<'_> {
    fn charge(&mut self, count: usize) -> Result<()> {
        if self.interrupt.is_some_and(|f| f()) {
            self.cancelled = true;
            return Err(ReadError::Limit);
        }
        self.remaining = self.remaining.checked_sub(count).ok_or(ReadError::Limit)?;
        Ok(())
    }
}

/// Certificate of the serialized shell and all actual support/pcurve bindings.
/// The caller supplies the authoritative source; no independent blind import
/// or decoded mass integration is implied by this source-known certificate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntryChamferProof {
    pub face_count: usize,
    /// Polynomial edge/support bindings; analytic circle checks are additional.
    pub certified_curve_bindings: usize,
    /// Maximum polynomial trim-to-support bound; analytic closures have
    /// separate extent/endpoint receipts bounded by tolerance/16.
    pub maximum_polynomial_trim_error_mm: f64,
}
/// Distinguishes invalid geometry, exhausted proof resources and cancellation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryChamferReadError {
    Invalid,
    Unsupported,
    Limit,
    Cancelled,
}
impl From<ReadError> for EntryChamferReadError {
    fn from(error: ReadError) -> Self {
        match error {
            ReadError::Invalid => Self::Invalid,
            ReadError::Unsupported => Self::Unsupported,
            ReadError::Limit => Self::Limit,
        }
    }
}
/// Prove the complete owned conical-entry shell against a known exact source.
/// Actual curves are checked on actual surfaces over their full knot domains;
/// a matching header, recent export or structural edge count is insufficient.
pub fn verify_thread_entry_chamfer(
    input: &str,
    body: &Chamfer,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
) -> std::result::Result<EntryChamferProof, EntryChamferReadError> {
    verify_thread_entry_chamfer_with_interrupt(input, body, origin, u, v, tolerance, None)
}
pub fn verify_thread_entry_chamfer_with_interrupt(
    input: &str,
    body: &Chamfer,
    origin: Point,
    u: Point,
    v: Point,
    tolerance: f64,
    interrupt: Option<&dyn Fn() -> bool>,
) -> std::result::Result<EntryChamferProof, EntryChamferReadError> {
    let mut work = Work {
        remaining: 512_000_000,
        interrupt,
        cancelled: false,
    };
    if work.charge(1).is_err() {
        return Err(EntryChamferReadError::Cancelled);
    }
    let plan =
        crate::step::thread_entry_chamfer::read_plan(body, [origin, u, v], tolerance, interrupt)
            .map_err(|e| match e {
                crate::step::EntryChamferStepError::Limit => EntryChamferReadError::Limit,
                crate::step::EntryChamferStepError::Unsupported => {
                    EntryChamferReadError::Unsupported
                }
                crate::step::EntryChamferStepError::Cancelled => EntryChamferReadError::Cancelled,
                _ => EntryChamferReadError::Invalid,
            })?;
    let result = compound::read_members_by(input, tolerance, true, true, |shell, id, _| {
        let graph = graph::Graph::read(shell, id, &mut work)?;
        require(graph.faces.len() == body.face_count() as usize)?;
        let mut proof = known::Verify {
            body,
            frame: [origin, u, v],
            tolerance: tolerance.min(1e-7),
            maximum: 0.,
            bound_curves: 0,
        };
        let cone = proof.cone(shell, graph.faces[plan.source.bands.len() + 2].surface)?;
        let (groups, _, tops) = proof.walls(shell, &graph, &plan, &cone, &mut work)?;
        proof.caps(shell, &graph, &plan, (&groups, &tops), &cone, &mut work)?;
        Ok(EntryChamferProof {
            face_count: graph.faces.len(),
            certified_curve_bindings: proof.bound_curves,
            maximum_polynomial_trim_error_mm: proof.maximum,
        })
    })
    .and_then(|mut p| p.pop().ok_or(ReadError::Invalid));
    if work.cancelled {
        Err(EntryChamferReadError::Cancelled)
    } else {
        result.map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn known_source_proves_actual_entry_shell_without_header_or_native_reader() {
        let tool = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let body = Chamfer::new(tool, 0.4, None).unwrap();
        let frame = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let [o, u, v] = frame;
        let text = crate::step::write_thread_entry_chamfer(&body, o, u, v, 1e-7).unwrap();
        let proof = verify_thread_entry_chamfer(&text, &body, o, u, v, 1e-7).unwrap();
        assert_eq!(proof.face_count, body.face_count() as usize);
        assert!(proof.certified_curve_bindings >= 20);
        assert!(proof.maximum_polynomial_trim_error_mm <= 5e-8);
    }
}

#[cfg(test)]
mod qualification {
    use super::*;
    fn source(left: bool, rounded: bool, starts: u32) -> crate::helical_thread::Thread {
        let root = rounded.then_some(1. / (4. * 3_f64.sqrt()));
        crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: root.map_or(0.5, |r| 0.875 * 3_f64.sqrt() / 2. - r),
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts,
            left_hand: left,
            root_radius_mm: root,
        })
        .unwrap()
    }
    #[test]
    fn physical_shell_matrix_keeps_round_roots_hands_starts_pose_contacts_and_terminal_necks() {
        for (left, rounded, starts, c, neck) in [
            (false, false, 1, 0.1, false),
            (true, false, 2, 0.7, true),
            (false, true, 3, 0.4, false),
            (true, true, 1, 0.8, true),
            (false, true, 1, 2., true),
        ] {
            let tail = neck.then_some(crate::thread_end_relief::Intent {
                width_mm: 0.6,
                radial_depth_mm: 0.1,
            });
            let body = Chamfer::new(source(left, rounded, starts), c, tail).unwrap();
            let [o, u, v] = [[2., -3., 5.], [0.8, 0.6, 0.], [0., 0., 1.]];
            let text = crate::step::write_thread_entry_chamfer(&body, o, u, v, 1e-7).unwrap();
            let result = verify_thread_entry_chamfer(&text, &body, o, u, v, 1e-7);
            assert!(
                result.is_ok(),
                "{left}/{rounded}/{starts}/{c}/{neck}: {result:?}"
            );
            let receipt = result.unwrap();
            assert_eq!(receipt.face_count, body.face_count() as usize);
            assert!(receipt.maximum_polynomial_trim_error_mm <= 5e-8);
        }
    }
    #[test]
    fn actual_rail_departure_cancelled_proof_and_invalid_inputs_never_publish_a_certificate() {
        let body = Chamfer::new(source(false, false, 1), 0.4, None).unwrap();
        let [o, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        let text = crate::step::write_thread_entry_chamfer(&body, o, u, v, 1e-7).unwrap();
        let entities = parse_entities(&text, 1e-7).unwrap();
        let mut shell = Shell::new(&entities, 1e-7, true);
        let solid = shell.represented_solids(true).unwrap()[0];
        let graph = graph::Graph::read(
            &mut shell,
            solid,
            &mut Work {
                remaining: 1_000_000,
                interrupt: None,
                cancelled: false,
            },
        )
        .unwrap();
        let wall = &graph.faces[0];
        let uses = helical::orient(&wall.wires[0], false);
        let rail = uses[uses.len() - 1].0;
        let raw = graph.edges[&rail].curve;
        let controls = refs(&shell.args(raw, "B_SPLINE_CURVE_WITH_KNOTS").unwrap()[2]).unwrap();
        let point = controls[controls.len() / 2];
        let p = shell.point(point).unwrap();
        let start = text.find(&format!("#{point}=")).unwrap();
        let end = start + text[start..].find(';').unwrap() + 1;
        let mut bad = text.clone();
        bad.replace_range(
            start..end,
            &format!(
                "#{point}=CARTESIAN_POINT('',({:.16E},{:.16E},{:.16E}));",
                p[0],
                p[1],
                p[2] + 0.02
            ),
        );
        assert!(verify_thread_entry_chamfer(&bad, &body, o, u, v, 1e-7).is_err());
        assert_eq!(
            verify_thread_entry_chamfer_with_interrupt(&text, &body, o, u, v, 1e-7, Some(&|| true)),
            Err(EntryChamferReadError::Cancelled)
        );
        let calls = std::cell::Cell::new(0usize);
        let callback = || {
            calls.set(calls.get() + 1);
            false
        };
        crate::step::thread_entry_chamfer::read_plan(&body, [o, u, v], 1e-7, Some(&callback))
            .unwrap();
        let after_source = calls.get() + 20;
        calls.set(0);
        let callback = || {
            calls.set(calls.get() + 1);
            calls.get() >= after_source
        };
        assert_eq!(
            verify_thread_entry_chamfer_with_interrupt(
                &text,
                &body,
                o,
                u,
                v,
                1e-7,
                Some(&callback)
            ),
            Err(EntryChamferReadError::Cancelled)
        );
        assert!(verify_thread_entry_chamfer(&text, &body, o, u, v, 1e-7).is_ok());
        assert!(verify_thread_entry_chamfer("broken", &body, o, u, v, 1e-7).is_err());
        assert_eq!(
            verify_thread_entry_chamfer(&text, &body, o, u, v, f64::NAN),
            Err(EntryChamferReadError::Invalid)
        );
        assert_eq!(
            verify_thread_entry_chamfer(&text, &body, o, [0.; 3], v, 1e-7),
            Err(EntryChamferReadError::Invalid)
        );
        let different = Chamfer::new(source(false, false, 1), 0.41, None).unwrap();
        assert!(verify_thread_entry_chamfer(&text, &different, o, u, v, 1e-7).is_err());
    }
}

#[cfg(test)]
fn with_shell<T>(data: &str, run: impl FnOnce(&mut Shell<'_, '_>) -> T) -> T {
    let file = format!(
        "ISO-10303-21;HEADER;FILE_DESCRIPTION(('test'),'2;1');FILE_NAME('','',(),(),'','','');FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));ENDSEC;DATA;{data}ENDSEC;END-ISO-10303-21;"
    );
    let entities = parse_entities(&file, 1e-7).unwrap();
    run(&mut Shell::new(&entities, 1e-7, true))
}

#[cfg(test)]
mod numeric_contracts {
    use super::*;
    #[test]
    fn typed_read_failures_distinguish_resource_uncertainty_and_cancelled_source_proof() {
        assert_eq!(
            EntryChamferReadError::from(ReadError::Unsupported),
            EntryChamferReadError::Unsupported
        );
        assert_eq!(
            EntryChamferReadError::from(ReadError::Limit),
            EntryChamferReadError::Limit
        );
        let tool = crate::helical_thread::Thread::new(crate::helical_thread::Spec {
            major_radius_mm: 3.,
            pitch_mm: 1.,
            length_mm: 6.6,
            depth_mm: 0.5,
            crest_width_mm: 0.125,
            lower_flank_deg: 30.,
            upper_flank_deg: 30.,
            starts: 1,
            left_hand: false,
            root_radius_mm: None,
        })
        .unwrap();
        let body = Chamfer::new(tool, 0.4, None).unwrap();
        let [o, u, v] = [[0.; 3], [1., 0., 0.], [0., 1., 0.]];
        assert_eq!(
            verify_thread_entry_chamfer("", &body, [1e15, 0., 0.], u, v, 1e-7),
            Err(EntryChamferReadError::Limit)
        );
        let calls = std::cell::Cell::new(0usize);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() == 2
        };
        assert_eq!(
            verify_thread_entry_chamfer_with_interrupt("", &body, o, u, v, 1e-7, Some(&cancel)),
            Err(EntryChamferReadError::Cancelled)
        );
        let tiny = Chamfer::from_material(tool, 3_f64.next_down(), 1e-300, None).unwrap();
        assert_eq!(
            verify_thread_entry_chamfer("", &tiny, o, u, v, 1e-7),
            Err(EntryChamferReadError::Unsupported)
        );
    }
}
