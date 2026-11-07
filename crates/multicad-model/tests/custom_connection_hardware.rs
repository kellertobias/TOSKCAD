use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::*,
    project_file::{self, ProjectFile},
    references::TopologyKind,
};
use multicad_kernel::{BooleanOperation, GeometryPolicy, HelicalThread, RigidTransform, Solid};
use multicad_model::Compiler;
use std::f64::consts::PI;
fn actor() -> Actor {
    Actor::local(Channel::Mcp)
}
fn compiler() -> Compiler {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
}
fn thread_volume(t: &ResolvedThread) -> f64 {
    let r = t.major_radius_mm;
    let root = r - t.profile.depth_mm;
    let w = t.profile.crest_width_mm;
    let p = t.pitch_mm;
    if let Some(radius) = t
        .metric_fit
        .as_ref()
        .and_then(|m| m.external_root_radius_mm)
    {
        let cr = root + radius;
        let x = radius * (PI / 3.).sin();
        let primitive = |x: f64| {
            (cr * cr + radius * radius) * x
                - x.powi(3) / 3.
                - cr * (x * (radius * radius - x * x).sqrt()
                    + radius * radius * (x / radius).asin())
        };
        let tangent = cr - radius / 2.;
        let squared =
            w * r * r + 2. / 3_f64.sqrt() * (r.powi(3) - tangent.powi(3)) / 3. + primitive(x)
                - primitive(-x);
        PI * t.length_mm * squared / p
    } else {
        let slopes = t.profile.lower_flank_deg.to_radians().tan()
            + t.profile.upper_flank_deg.to_radians().tan();
        let root_width = p - w - (r - root) * slopes;
        PI * t.length_mm
            * (w * r * r + slopes * (r.powi(3) - root.powi(3)) / 3. + root_width * root * root)
            / p
    }
}
fn material(shape: &Solid, p: [f64; 3]) -> f64 {
    let e = 0.002;
    let probe = Solid::box_solid([e; 3], GeometryPolicy::default())
        .unwrap()
        .transformed(RigidTransform {
            translation_mm: p.map(|x| x - e / 2.),
            ..Default::default()
        })
        .unwrap();
    shape
        .boolean(&probe, BooleanOperation::Intersect, Default::default())
        .unwrap()
        .map_or(0., |s| s.volume_mm3().unwrap())
        / e.powi(3)
}
fn defaults() -> [MechanicalPartParameters; 3] {
    [
        default_custom_connection_wood_screw(),
        default_custom_connection_wood_insert(),
        default_custom_connection_hanger_bolt(),
    ]
}
#[test]
fn all_three_engineered_hardware_forms_have_actual_helices_independent_mass_material_and_step() {
    let dir = tempfile::tempdir().unwrap();
    for (case, mut source) in defaults().into_iter().enumerate() {
        let mut s = Session::new(Document::new("Native custom hardware").unwrap()).unwrap();
        let MechanicalPartParameters::CustomConnection { parameters } = &mut source else {
            panic!()
        };
        match parameters.as_mut() {
            CustomConnectionHardwareParameters::WoodScrew { frame, .. }
            | CustomConnectionHardwareParameters::WoodInsert { frame, .. }
            | CustomConnectionHardwareParameters::HangerBolt { frame, .. } => {
                *frame = PrimitiveFrame {
                    origin: [length(20.), length(-7.), length(9.)],
                    u: [0., 1., 0.],
                    v: [0., 0., 1.],
                }
            }
        };
        let resolved = source.resolve(s.parameters()).unwrap();
        let ResolvedMechanicalPart::CustomConnection { parameters } = resolved else {
            panic!()
        };
        let (expected, void) = match parameters.as_ref() {
            ResolvedCustomConnectionHardware::WoodScrew {
                wood_thread,
                head,
                plain_length_mm,
                plain_radius_mm,
                ..
            } => {
                let ResolvedScrewHead::ExternalHex {
                    across_flats_mm,
                    height_mm,
                } = head
                else {
                    panic!()
                };
                (
                    thread_volume(wood_thread)
                        + 3_f64.sqrt() / 2. * across_flats_mm.powi(2) * height_mm
                        + PI * plain_radius_mm.powi(2) * plain_length_mm,
                    None,
                )
            }
            ResolvedCustomConnectionHardware::WoodInsert {
                outer_thread,
                inner_thread,
                inner_thread_start,
                ..
            } => (
                thread_volume(outer_thread) - thread_volume(inner_thread),
                Some(std::array::from_fn(|i| {
                    inner_thread_start.origin_mm[i] + inner_thread_start.normal()[i] * 2.
                })),
            ),
            ResolvedCustomConnectionHardware::HangerBolt {
                wood_thread,
                metric_thread,
                plain_radius_mm,
                plain_length_mm,
                ..
            } => (
                thread_volume(wood_thread)
                    + thread_volume(metric_thread)
                    + PI * plain_radius_mm.powi(2) * plain_length_mm,
                None,
            ),
        };
        let r = MechanicalPartRequest::new(format!("Custom{case}"), source);
        s.commit(
            prepare_mechanical_part(&s, 0, actor(), &r)
                .unwrap()
                .prepared,
        )
        .unwrap();
        let model = compiler().compile(s.document(), s.revision()).unwrap();
        let shape = model.solid(r.feature).unwrap();
        assert_eq!(shape.solid_count().unwrap(), 1);
        let actual = shape.volume_mm3().unwrap();
        let area = shape
            .faces()
            .unwrap()
            .iter()
            .map(|f| f.area_mm2)
            .sum::<f64>();
        let budget = 1e-7 * area + 1e-7 * actual;
        assert!(
            (actual - expected).abs() <= budget,
            "case{case} native{actual} analytic{expected} budget{budget}"
        );
        if let Some(at) = void {
            assert!(material(shape, at) < 1e-6);
        }
        let topology = shape.topology_snapshot(r.feature, s.revision()).unwrap();
        assert!(
            topology
                .elements
                .iter()
                .filter(|e| e.key.kind == TopologyKind::Face)
                .all(|e| !e.origins.is_empty())
        );
        assert!(
            topology
                .elements
                .iter()
                .any(|e| e.origins.iter().any(|o| o.role.starts_with("thread/")))
        );
        let step = dir.path().join(format!("custom-{case}.step"));
        shape.write_step(&step).unwrap();
        let back = Solid::read_step(&step, Default::default()).unwrap();
        assert_eq!(back.solid_count().unwrap(), 1);
        assert!((back.volume_mm3().unwrap() - expected).abs() <= budget);
        let before = s.document().clone();
        let mut bytes = vec![];
        project_file::write(
            &ProjectFile::new(before.clone()).unwrap(),
            &mut bytes,
            Default::default(),
        )
        .unwrap();
        let reopened = project_file::read(
            &mut std::io::Cursor::new(bytes),
            Default::default(),
            &project_file::supported_capabilities(),
        )
        .unwrap();
        let fresh = compiler().compile(&reopened.project.document, 0).unwrap();
        assert!((fresh.solid(r.feature).unwrap().volume_mm3().unwrap() - actual).abs() <= budget);
        assert_eq!(*s.document(), before);
    }
}
#[test]
fn independent_owners_and_wood_tip_material_preserve_completed_source_geometry() {
    let mut s = Session::new(Document::new("Owner-safe custom screw").unwrap()).unwrap();
    let raw = default_custom_wood_thread()
        .resolve(s.parameters(), ThreadMode::NewExternal)
        .unwrap();
    let cached_thread = Solid::helical_thread_named(
        HelicalThread {
            major_radius_mm: raw.major_radius_mm,
            pitch_mm: raw.pitch_mm,
            length_mm: raw.length_mm,
            depth_mm: raw.profile.depth_mm,
            crest_width_mm: raw.profile.crest_width_mm,
            lower_flank_deg: raw.profile.lower_flank_deg,
            upper_flank_deg: raw.profile.upper_flank_deg,
            radial_taper: 0.,
            starts: 1,
            left_hand: false,
        },
        EntityId::new(),
        GeometryPolicy::default(),
    )
    .unwrap();
    let thread_before = cached_thread.to_brep().unwrap();
    let r = MechanicalPartRequest::new("First custom", default_custom_connection_wood_screw());
    s.commit(
        prepare_mechanical_part(&s, 0, actor(), &r)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let first = compiler().compile(s.document(), s.revision()).unwrap();
    let shape = first.solid(r.feature).unwrap();
    let brep = shape.to_brep().unwrap();
    let topology = shape.topology_snapshot(r.feature, 0).unwrap();
    let near_flat = [0.5, 0., 11.98];
    assert!(material(shape, near_flat) > 0.999);
    let other = MechanicalPartRequest::new("Second custom", r.parameters.clone());
    s.commit(
        prepare_mechanical_part(&s, s.revision(), actor(), &other)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let second = compiler().compile(s.document(), s.revision()).unwrap();
    assert_eq!(shape.to_brep().unwrap(), brep);
    assert_eq!(cached_thread.to_brep().unwrap(), thread_before);
    assert_eq!(shape.topology_snapshot(r.feature, 0).unwrap(), topology);
    let othertopology = second
        .solid(other.feature)
        .unwrap()
        .topology_snapshot(other.feature, 0)
        .unwrap();
    assert!(
        othertopology
            .elements
            .iter()
            .flat_map(|e| &e.origins)
            .all(|o| o.source == other.feature)
    );
    let mut edited = r.parameters.clone();
    if let MechanicalPartParameters::CustomConnection { parameters } = &mut edited
        && let CustomConnectionHardwareParameters::WoodScrew { plain_length, .. } =
            parameters.as_mut()
    {
        *plain_length = length(2.);
    }
    let before = s.document().clone();
    s.commit(
        prepare_mechanical_part_edit(&s, s.revision(), actor(), r.feature, &edited)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let changed = compiler().compile(s.document(), s.revision()).unwrap();
    assert!(
        changed.solid(r.feature).unwrap().bounds().unwrap().min[2]
            < second.solid(r.feature).unwrap().bounds().unwrap().min[2] - 1.9
    );
    assert_eq!(
        changed.solid(other.feature).unwrap().volume_mm3().unwrap(),
        second.solid(other.feature).unwrap().volume_mm3().unwrap()
    );
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(*s.document(), before);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(
        read_mechanical_part(s.document(), r.feature)
            .unwrap()
            .output
            .body,
        r.body
    );
}
#[test]
fn unsupported_cosmetic_or_fabricated_wood_source_is_atomic() {
    let s = Session::new(Document::new("No fake wood manufacturing").unwrap()).unwrap();
    let mut p = default_custom_connection_hanger_bolt();
    let MechanicalPartParameters::CustomConnection { parameters } = &mut p else {
        panic!()
    };
    if let CustomConnectionHardwareParameters::HangerBolt { wood_thread, .. } = parameters.as_mut()
    {
        wood_thread.family = ThreadFamily::IsoMetric;
        wood_thread.custom = None;
    }
    let r = MechanicalPartRequest::new("Invalid", p);
    let before = s.document().clone();
    assert!(prepare_mechanical_part(&s, 0, actor(), &r).is_err());
    assert_eq!(*s.document(), before);
}

#[test]
fn tiny_insert_floor_is_refused_by_native_policy_without_publishing_source() {
    let s = Session::new(Document::new("Native minimum insert floor").unwrap()).unwrap();
    let mut p = default_custom_connection_wood_insert();
    if let MechanicalPartParameters::CustomConnection { parameters } = &mut p
        && let CustomConnectionHardwareParameters::WoodInsert { inner_thread, .. } =
            parameters.as_mut()
    {
        inner_thread.length = length(4. - 1e-6);
    }
    let r = MechanicalPartRequest::new("Too thin floor", p);
    let original = s.document().clone();
    let prepared = prepare_mechanical_part(&s, 0, actor(), &r).unwrap();
    assert!(compiler().compile(prepared.prepared.document(), 1).is_err());
    assert_eq!(*s.document(), original);
}
