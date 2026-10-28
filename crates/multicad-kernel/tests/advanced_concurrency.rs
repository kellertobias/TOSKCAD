//! Independent owner threads share no native handles or builders. This binary
//! has one test, so its first run also exercises concurrent native first use.
use multicad_core::{document::EntityId, references::*};
use multicad_kernel::*;
use std::sync::{Arc, Barrier};
fn origins(n: usize) -> Vec<Origin> {
    (0..n)
        .map(|_| Origin {
            source: EntityId::new(),
            role: "curve".into(),
        })
        .collect()
}
fn rect(size: f64, z: f64) -> CurvedProfile {
    let points = [[0., 0.], [size, 0.], [size, size], [0., size]];
    CurvedProfile {
        frame: PlaneFrame {
            origin: [0., 0., z],
            ..Default::default()
        },
        outer: (0..4)
            .map(|i| CurvedEdge2d::Line {
                a: points[i],
                b: points[(i + 1) % 4],
            })
            .collect(),
        holes: vec![],
    }
}
#[test]
fn independent_owner_advanced_operations_are_bounded_and_source_neutral() {
    let barrier = Arc::new(Barrier::new(4));
    let owners:Vec<_>=(0..4).map(|_| {let barrier=barrier.clone();std::thread::spawn(move || {
        let owner=EntityId::new();let policy=GeometryPolicy::default();
        let source=Solid::box_named([20.,12.,8.],owner,policy);
        barrier.wait(); // Every owner reaches this even if input construction failed.
        let source=source.unwrap();let original=source.topology_snapshot(owner,0).unwrap();
        let face_key=|normal:[f64;3]|original.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Plane{frame}) if frame.normal().into_iter().zip(normal).all(|(a,b)|(a-b).abs()<1e-9))).unwrap().key;
        for round in 0..10 {
            let feature=EntityId::new();
            match round%5 {
                0=> {let face=TreatmentFace{key:face_key([0.,0.,1.]),origin:Origin{source:feature,role:"face/opening".into()}};let shell=source.shell_named(&[face],1.,feature,policy).unwrap();assert!(shell.volume_mm3().unwrap()>0.);}
                1=> {let face=TreatmentFace{key:face_key([1.,0.,0.]),origin:Origin{source:feature,role:"face/draft".into()}};let draft=source.draft_faces_named(&[face],[0.,0.,1.],PlaneFrame::default(),3f64.to_radians(),feature,policy).unwrap();assert!(draft.volume_mm3().unwrap()>0.);}
                2=> {let profile=rect(2.,0.);let swept=Solid::sweep_polyline_named(&profile,&[[0.;3],[0.,0.,10.]],feature,&origins(4),&origins(1),policy).unwrap();assert!(swept.volume_mm3().unwrap()>0.);let loft=Solid::loft_profiles_named(&[profile,rect(3.,10.)],feature,&[origins(4),origins(4)],false,policy).unwrap();assert!(loft.volume_mm3().unwrap()>0.);}
                3=> {let mirror=source.mirrored_named(PlaneFrame{origin:[30.,0.,0.],u:[0.,1.,0.],v:[0.,0.,1.]},feature,policy).unwrap().with_member_origin(feature,0).unwrap();let shifted=source.transformed(RigidTransform{translation_mm:[10.,0.,0.],..Default::default()}).unwrap().with_member_origin(feature,1).unwrap();let compound=Solid::compound_named(&[&mirror,&shifted],feature,policy).unwrap();assert_eq!(compound.solid_count().unwrap(),2);}
                _=> {let key=original.elements.iter().find(|e|matches!(e.geometry,Some(AnalyticGeometry::Line{a_mm:a,b_mm:b}) if a[0]==20.&&b[0]==20.&&a[1]==0.&&b[1]==0.)).unwrap().key;let edge=SidedTreatmentEdge{edge:TreatmentEdge{key,origin:Origin{source:feature,role:"edge/chamfer".into()}},reference_face:face_key([1.,0.,0.])};let chamfer=source.chamfer_edges_sided_named(&[edge],SidedChamfer::Distances{first_mm:1.,second_mm:2.},feature,policy).unwrap();assert!(chamfer.volume_mm3().unwrap()>0.);}
            }
            assert_eq!(source.topology_snapshot(owner,0).unwrap(),original);
        }
    })}).collect();
    for owner in owners {
        owner.join().expect("native advanced owner failed");
    }
}
