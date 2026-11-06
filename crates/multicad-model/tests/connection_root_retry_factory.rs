//! A failed first native union is never the retained geometry. Only the fully
//! guarded retry may enter the owner-local factory; each hit gets fresh names.
use multicad_core::{
    commands::Session,
    document::{Document, EntityId},
    model::{ThreadFit, ThreadHand, ThreadMode, ThreadParameters, length},
    references::Origin,
};
use multicad_kernel::{
    GeometryPolicy, HelicalThread, RootRoundedHelicalThread, Solid, thread_factory_stats,
};

#[test]
fn symmetric_root_retry_retains_only_the_validated_result_and_rebinds_cache_owners() {
    let session = Session::new(Document::new("Root retry cache proof").unwrap()).unwrap();
    let resolved = ThreadParameters {
        major_diameter: length(3.),
        pitch: length(0.5),
        length: length(4.),
        fit: ThreadFit::Metric6gExternal,
        hand: ThreadHand::Left,
        ..Default::default()
    }
    .resolve(session.parameters(), ThreadMode::NewExternal)
    .unwrap();
    let spec = RootRoundedHelicalThread {
        profile: HelicalThread {
            major_radius_mm: resolved.major_radius_mm,
            pitch_mm: resolved.pitch_mm,
            length_mm: resolved.length_mm,
            depth_mm: resolved.profile.depth_mm,
            crest_width_mm: resolved.profile.crest_width_mm,
            lower_flank_deg: resolved.profile.lower_flank_deg,
            upper_flank_deg: resolved.profile.upper_flank_deg,
            radial_taper: resolved.profile.radial_taper,
            starts: resolved.starts,
            left_hand: true,
        },
        root_radius_mm: resolved
            .metric_fit
            .as_ref()
            .unwrap()
            .external_root_radius_mm
            .unwrap(),
    };
    let before = thread_factory_stats().unwrap();
    assert_eq!(before.entries, 0);
    let a = EntityId::new();
    let b = EntityId::new();
    let first =
        Solid::root_rounded_helical_thread_named(spec, a, GeometryPolicy::default()).unwrap();
    assert_eq!(first.solid_count().unwrap(), 1);
    let cold = thread_factory_stats().unwrap();
    assert_eq!(cold.constructions - before.constructions, 1);
    assert_eq!(cold.bounded_attempts - before.bounded_attempts, 2);
    assert_eq!(cold.continuous_attempts - before.continuous_attempts, 0);
    assert_eq!(cold.entries, 1);
    let second =
        Solid::root_rounded_helical_thread_named(spec, b, GeometryPolicy::default()).unwrap();
    let warm = thread_factory_stats().unwrap();
    assert_eq!(warm.hits - cold.hits, 1);
    assert_eq!(warm.constructions, cold.constructions);
    assert_eq!(warm.bounded_attempts, cold.bounded_attempts);
    assert_eq!(warm.entries, 1);
    assert_eq!(first.volume_mm3().unwrap(), second.volume_mm3().unwrap());
    assert_eq!(first.bounds().unwrap(), second.bounds().unwrap());
    let mut expected = first.topology_snapshot(a, 0).unwrap();
    let actual = second.topology_snapshot(b, 0).unwrap();
    expected.owner = b;
    let renamed = |origin: &Origin| {
        assert_eq!(origin.source, a);
        Origin {
            source: b,
            role: origin.role.clone(),
        }
    };
    for element in &mut expected.elements {
        element.origins = element.origins.iter().map(renamed).collect();
        element.boundary = element.boundary.iter().map(renamed).collect();
        for frame in &mut element.frames {
            frame.origin = renamed(&frame.origin);
        }
    }
    assert_eq!(expected, actual);
}
