# Wood screw dimensional source notices

The [curation](../../wood-screws-curation.json) retains forty-four discrete dimensional references from Bitork Fasteners' undated DIN 571 hexagon-head wood screw sheet. It covers every populated length cell for the selected requested diameters 4, 5, 6, 8 and 10 mm. The [source ledger](source-ledger.json) records original public URLs, exact inspected file SHA256 values and source limits. No manufacturer PDF, diagram, photograph, logo, CAD model or implementation code is included.

## Sources and selected dimensions

[Bitork's manufacturer sheet](https://www.bitorkfasteners.com/_files/ugd/179fe5_75017d246622499e8e22164ae1b4ed59.pdf), PDF page 1, supplies the head dimensions, discrete diameter/length/weight cells and bearing-face-to-tip length convention. No publication date or specific DIN edition was identified in the sheet. The separate DIN publisher record identifies DIN 571:2016-12; that identity does not establish that every requirement of that edition was inspected or reproduced.

The source's nominal `k` is the selected head-height reference; `s` max is the selected across-flats reference. Their published minimum/maximum bounds remain provenance. The diagonal `e` row is not labeled minimum or maximum and remains an unqualified diagonal reference. An ideal regular hexagon derives its corners from across flats. The diagonal value is not separately imposed on that ideal geometry. Length uses the published nominal `l`, with published minimum/maximum length bounds retained separately. Source weight is provenance, not a modeled mass, material assignment or guaranteed product weight.

[Industrias Placencia's technical catalog](https://www.tornilleriaplacencia.com/temas/bootstrap/uploads/archivos/Apartado_Tecnico.pdf), PDF page 18, section 1.3.2, separately supplies DIN 7998 wood V-reference pitch, crest/core diameter notation, an included angle of 60°±6° and manufacturer-selected thread ends. Its drawing identifies the actual screw core diameter, not a pilot-drill recommendation. The source uses h15 notation, but no diameter tolerance band or fit model is derived here. No publication date or exact crest/root truncation, radius, point angle or run-in construction was established.

[Westfield's own supplier specification](https://www.westfieldfasteners.co.uk/Standards/Woodscrew-CoachScr.pdf), Figure 1, corroborates the minimum threaded extent `b ≥ 0.6l`, including point and thread-start transition. This is a minimum zone, not an exact actual thread length, full-depth helical span or thread-start position. Westfield is a supplier reference, not a replacement for a complete manufacturer or standards drawing. Its contradictory diameter-5 maximum head-height cell is not used; the curation takes Bitork's independently printed value.

## Representation and availability

Every selected preset uses only `SimplifiedEnvelope`. A connected nominal-diameter stock envelope and external-hex head retain the sourced mounting and underhead length convention. The flat distal stock plane conservatively encloses the unspecified manufacturer point. It does not model a conical point, wood helix, root/crest truncations, run-in or material removal. The reference pitch/core/angle and minimum zone remain source intent. No metric M-thread, metric pitch diameter, fit class, hand or starts are inferred. Application manufacturing export is unavailable for this simplified representation.

The curation records all twelve requested diameters. Diameter 7 has reference head/profile dimensions but no populated Bitork length cell, so it has no preset. Diameters 2, 2.5, 3, 3.5, 4.5 and 9 have no selected Bitork preset. An unavailable status describes this curated family, not universal commercial availability.

Fischer PowerFast II CTF BC has real TX product rows in catalog 506632 · 01/2026, but complete head-height, recess-depth and proprietary point/thread geometry were not established. It is excluded. Its proprietary thread must not inherit DIN 7998 reference dimensions. Phillips H is also excluded because no complete pinned product-specific dimensional/length/recess/point row was established. PZ/Z is a different drive and is not substituted for H.

Custom dimension overrides must retain the immutable reference facts while clearing dimensional matching. They do not become verified stock sizes. No material, grade, coating, current-standard compliance, strength, connection capacity, physical fit or print-success claim is transferred from a source product.

## Rights

These independently transcribed numerical references and source citations do not import manufacturer geometry or artwork. No general redistribution license for the source PDFs, diagrams, logos or CAD files was established; they are not included in the Library. Public access does not grant rights to redistribute those works. Source copyright and trademark rights remain with their respective holders. No supplier endorsement is claimed.

No FreeCAD or BOLTS code or dataset is adapted for these presets. Their licenses do not grant rights to the separate manufacturer publications. There is no LGPL attribution applied to the new factual curation merely because other Library families use licensed BOLTS data.
