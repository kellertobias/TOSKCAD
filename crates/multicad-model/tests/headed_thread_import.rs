use multicad_core::{
    commands::{Actor, Channel, Session},
    document::{Document, EntityId},
    model::{
        self, MechanicalPartParameters, MechanicalPartRequest, ScrewDrive, ScrewHead,
        ThreadRepresentation, length,
    },
    project_file::{self, ProjectFile},
};
use multicad_kernel::Solid;
use multicad_model::{Compiler, GeometryBackend};
fn compile(s: &Session) -> multicad_model::Model {
    Compiler {
        omit_display: true,
        ..Default::default()
    }
    .compile_with_backend(s.document(), s.revision(), GeometryBackend::Rust, None)
    .unwrap()
}
#[test]
fn saved_hex_screw_edits_and_history_retain_blind_step_importable_geometry() {
    saved_screw(true);
}
#[test]
fn saved_socket_screw_edits_and_history_retain_blind_step_importable_geometry() {
    saved_screw(false);
}
fn saved_screw(hex: bool) {
    let actor = || Actor::local(Channel::Mcp);
    let mut s = Session::new(Document::new("Importable modeled hex screw").unwrap()).unwrap();
    let mut parameters = model::default_machine_screw();
    let MechanicalPartParameters::MachineScrew {
        head,
        drive,
        thread,
        underhead_length,
        shank_diameter,
        ..
    } = &mut parameters
    else {
        panic!()
    };
    *head = if hex {
        ScrewHead::ExternalHex {
            across_flats: length(10.),
            height: length(4.),
        }
    } else {
        ScrewHead::SocketCap {
            diameter: length(11.),
            height: length(4.),
        }
    };
    *drive = if hex {
        ScrewDrive::ExternalHex
    } else {
        ScrewDrive::HexSocket {
            across_flats: length(4.),
            depth: length(2.),
        }
    };
    *underhead_length = Some(length(8.));
    *shank_diameter = length(6.);
    thread.major_diameter = length(6.);
    thread.pitch = length(1.);
    thread.length = length(8.);
    thread.representation = ThreadRepresentation::Modeled;
    let request = MechanicalPartRequest::new("M6 hex", parameters.clone());
    s.commit(
        model::prepare_mechanical_part(&s, s.revision(), actor(), &request)
            .unwrap()
            .prepared,
    )
    .unwrap();
    let original = s.document().clone();
    let MechanicalPartParameters::MachineScrew { head, .. } = &mut parameters else {
        panic!()
    };
    *head = if hex {
        ScrewHead::ExternalHex {
            across_flats: length(10.),
            height: length(5.),
        }
    } else {
        ScrewHead::SocketCap {
            diameter: length(11.),
            height: length(5.),
        }
    };
    s.commit(
        model::prepare_mechanical_part_edit(
            &s,
            s.revision(),
            actor(),
            request.feature,
            &parameters,
        )
        .unwrap()
        .prepared,
    )
    .unwrap();
    let edited = s.document().clone();
    s.undo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &original);
    s.redo(s.revision(), actor()).unwrap();
    assert_eq!(s.document(), &edited);
    let mut bytes = vec![];
    project_file::write(
        &ProjectFile::new(edited).unwrap(),
        &mut bytes,
        Default::default(),
    )
    .unwrap();
    let file = project_file::read(
        &mut bytes.as_slice(),
        Default::default(),
        &project_file::supported_capabilities(),
    )
    .unwrap()
    .project;
    let reopened = Session::new(file.document).unwrap();
    let model = compile(&reopened);
    let source = model.solid(request.feature).unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let path = scratch.path().join("hex.step");
    source.write_step(&path).unwrap();
    let imported = Solid::import_rust_step(
        &std::fs::read_to_string(&path).unwrap(),
        EntityId::new(),
        Compiler::default().policy,
    )
    .unwrap();
    assert!((imported.volume_mm3().unwrap() - source.volume_mm3().unwrap()).abs() < 1e-6);
    assert_eq!(imported.faces().unwrap().len(), if hex { 13 } else { 15 });
}
