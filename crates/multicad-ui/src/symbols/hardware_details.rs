use crate::egui;
use multicad_core::{
    model::{ResolvedCustomConnectionHardware, ResolvedMechanicalPart, ScrewLengthConvention},
    symbols::ConfiguredHardwarePart,
};

fn mm(value: f64) -> String {
    format!("{value:.3}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn lengths(part: &ResolvedMechanicalPart) -> String {
    match part {
        ResolvedMechanicalPart::Washer {
            bore_radius_mm,
            outside_radius_mm,
            thickness_mm,
            ..
        } => format!(
            "Ø{} bore × Ø{} outside × {} thick mm",
            mm(2. * bore_radius_mm),
            mm(2. * outside_radius_mm),
            mm(*thickness_mm)
        ),
        ResolvedMechanicalPart::BearingEnvelope {
            bore_radius_mm,
            outside_radius_mm,
            width_mm,
            ..
        } => format!(
            "Ø{} bore × Ø{} outside × {} wide mm",
            mm(2. * bore_radius_mm),
            mm(2. * outside_radius_mm),
            mm(*width_mm)
        ),
        ResolvedMechanicalPart::HexNut {
            across_flats_mm,
            thread,
        } => format!(
            "Ø{} nominal, {} pitch, {} high, {} across flats mm",
            mm(2. * thread.nominal_major_radius_mm),
            mm(thread.pitch_mm),
            mm(thread.length_mm),
            mm(*across_flats_mm)
        ),
        ResolvedMechanicalPart::MachineScrew {
            authored_length_mm,
            length_convention,
            thread,
            ..
        } => format!(
            "Ø{} nominal, {} pitch, {} {} mm",
            mm(2. * thread.nominal_major_radius_mm),
            mm(thread.pitch_mm),
            mm(*authored_length_mm),
            length_basis(*length_convention)
        ),
        ResolvedMechanicalPart::WoodScrew { parameters } => format!(
            "Ø{} nominal, {} {} mm · {} mm reference pitch",
            mm(parameters.nominal_diameter_mm),
            mm(parameters.authored_length_mm),
            length_basis(parameters.length_convention),
            mm(parameters.reference_profile.pitch_mm)
        ),
        ResolvedMechanicalPart::CustomConnection { parameters } => match parameters.as_ref() {
            ResolvedCustomConnectionHardware::WoodScrew { wood_thread, .. } => format!(
                "Ø{} wood thread, {} pitch, {} long mm",
                mm(2. * wood_thread.major_radius_mm),
                mm(wood_thread.pitch_mm),
                mm(wood_thread.length_mm)
            ),
            ResolvedCustomConnectionHardware::WoodInsert {
                outer_thread,
                inner_thread,
                ..
            } => format!(
                "Ø{} wood thread, Ø{} nominal inner thread mm",
                mm(2. * outer_thread.major_radius_mm),
                mm(2. * inner_thread.nominal_major_radius_mm)
            ),
            ResolvedCustomConnectionHardware::HangerBolt {
                wood_thread,
                metric_thread,
                ..
            } => format!(
                "Ø{} wood thread, Ø{} nominal metric thread mm",
                mm(2. * wood_thread.major_radius_mm),
                mm(2. * metric_thread.nominal_major_radius_mm)
            ),
        },
    }
}

fn length_basis(convention: ScrewLengthConvention) -> &'static str {
    match convention {
        ScrewLengthConvention::Underhead => "underhead",
        ScrewLengthConvention::Overall => "overall",
    }
}

pub(super) fn show(ui: &mut egui::Ui, parts: &[ConfiguredHardwarePart]) {
    for part in parts {
        ui.push_id(("hardware-attribution", part.feature), |ui| {
            let attribution = &part.attribution;
            ui.separator();
            ui.strong(attribution.basis.label());
            ui.label(format!("{} · {}", part.name, part.family));
            ui.small(format!("Draft dimensions: {}", lengths(&part.current)));
            ui.small(format!(
                "Assigned material: {}",
                attribution
                    .actual_material
                    .as_deref()
                    .unwrap_or("Unassigned")
            ));
            if let Some(reference) = &attribution.reference {
                ui.small(format!(
                    "Retained catalog reference: {} · {}",
                    reference.designation, reference.revision
                ));
                ui.small(format!("Reference dimensions: {}", lengths(&part.retained)));
                ui.small(format!(
                    "Catalog material: {}",
                    reference.material.as_deref().unwrap_or("Not stated")
                ));
            } else if part.current != part.retained {
                ui.small(format!(
                    "Pinned default dimensions: {}",
                    lengths(&part.retained)
                ));
            }
            ui.collapsing("Mounting datum", |ui| {
                let mount = part.current.mount_pose();
                let [x, y, z] = mount.translation_mm.map(mm);
                let [u, v, w] = mount.vector([0., 0., 1.]).map(mm);
                ui.small(format!("Definition origin: ({x}, {y}, {z}) mm"));
                ui.small(format!("Axis direction: ({u}, {v}, {w})"));
            });
        });
    }
}
