# Machine-screw dimensional data notices

The machine-screw dimensional dataset is adapted from `hexsocketheadcap` in the accompanying unmodified `hex_socket.blt` and the DIN 933 rows in unmodified `hex.blt`. Both original files identify Johannes Reinhardt, copyright 2013, and grant the GNU Lesser General Public License, version 2.1 or any later version. Their notices and warranty disclaimers remain intact; the complete license is in [COPYING.LESSER](COPYING.LESSER).

The exact upstream commit, source URLs and SHA256 hashes are recorded in [machine-screws-curation.json](../../machine-screws-curation.json). Its adapted dimensional data and corresponding dimensional data retained in the native screw projects are provided under LGPL-2.1-or-later. The original files, adapted curation source, this notice and the license accompany the bundled Library. There is no warranty of merchantability or fitness for a particular purpose.

## Adaptation notice, 2026-11-03

MultiCAD's catalog maintainers selected nineteen discrete metric machine-screw presets: fourteen socket-cap/internal-hex rows and five external-hex rows. The curation owns each nominal diameter, coarse pitch, underhead length, thread length, head/drive dimensions, dimensional limits, source reference and modeling modification. These are dimensional presets with material, grade, coating and stock unspecified. Source product identifiers identify dimensional rows and do not transfer a supplier's product certification.

### Socket-cap heads and internal hex drives

The [HPC 2025 catalog](https://shop.hpceurope.com/pdf/fr/CHCSS.pdf), printed pages 554–557 / PDF pages 1–4, supplies the explicit selected CHC SKU length/thread-length rows and outer head envelopes. The [NBK excerpt](https://www.nbk1560.com/images/en-US/product/technical_data/rokkaku_bolt_NBK/rokkaku_bolt_NBK_1.pdf), PDF page 1, identifies ISO 4762:2004 and JIS B 1176:2015. It distinguishes actual socket across-flats limits from nominal key sizes, minimum straight socket depth `t`, minimum remaining dimension `w`, head-height limits and separate plain/knurled head-diameter maxima.

The actual recess across flats uses the midpoint of the named NBK range as an engineering target. It is not a prescribed standard dimension. The model uses a true six-sided recess with a flat floor at the minimum straight depth. Its remaining floor, head height minus recess depth, exceeds the listed `w` minimum for every selected row. This numeric check does not reproduce the source's drilled or conical bottom. Socket-bottom tooling and mouth treatments are omitted.

HPC's stated head diameter is retained as an outer-diameter envelope with surface detail unspecified. M3–M10 values numerically match NBK's knurled maxima, while M2/M2.5 match the plain maxima. Numeric agreement does not establish a surface treatment; neither a plain nor a knurled SKU variant is asserted. Head height uses the named maximum.

Explicit HPC SKU thread lengths take precedence over a threshold formula. HPC lists full threads at M3×20, M4×25, M5×25, M6×30, M8×35 and M10×40, whereas the pinned BOLTS text describes full threading strictly below those thresholds. The selected full and partial rows retain their published `B` values; no equality inference or Cartesian diameter/length stock matrix is used.

### External hex heads

The selected [Norelem 07171 dimensional rows](https://www.norelem.it/en/Product-overview/Flexible-standard-component-system/07000/Nuts-screws-washers-securing-elements/Hexagon-head-bolts-DIN-933-ISO-4017/Hexagon-head-bolts-DIN-933-ISO-4017-steel/p/agid.19862), PDF pages 1–2, identify full-thread DIN 933 products. No dated standard edition was identified for that source. The separate [Nufast drawing](https://www.nufast.co.uk/uploads/DIN_933.pdf) explicitly names DIN 933:1987 and is dated 2014-06-25. Its M5/M6/M8/M10 across-flats and head-height ranges and minimum corner diameters provide a dimensional cross-check; that date and edition do not date the Norelem product. M4 is absent from the Nufast drawing, so its record retains Norelem dimensional reference points without invented tolerance limits.

DIN M10 across flats is 17 mm in these selected rows. The BOLTS ISO M10 variants combine 16 mm across flats with the DIN minimum corner diameter 18.90 mm. An ideal regular hexagon at 16 mm across flats has a corner diameter of about 18.475 mm, so that hybrid is rejected. It is not used as an ISO 4017 preset or relabeled as DIN geometry.

The external head is a sharp regular hexagonal prism at the stated across flats. Its ideal corner diameter follows from that geometry; the source corner dimension remains a cross-check. BOLTS unthreaded `h` detail is omitted in the selected full-thread convention.

## Geometry and coverage boundaries

The native projects retain one frame at the underhead plane, with the head behind that plane and the shaft toward the tip. Underhead length and thread length are separate authored dimensions. A partial thread leaves a plain shank beside the head; a full-length cylinder is not added over the thread grooves. Threads use the shared metric nominal basic profile. ISO 965 fit limits are not implemented.

The independently authored solids and native previews omit underhead fillets, head/tip chamfers, thread runout, surface detail and dimensional tolerances. Their sharp thread transitions and flat socket floors are bounded modeling conventions. The source metadata does not certify current-standard compliance, gage/physical fit, strength, torque or stock availability. No manufacturer CAD, meshes, artwork, logos or catalog prose are redistributed.

All twelve requested nominal sizes have explicit per-family coverage statuses in the curation. M3.5, M4.5, M7 and M9 have no preset in this selected matrix; external-hex M2, M2.5 and M3 also remain unavailable/custom cases here. An unavailable status describes this curated matrix and does not assert commercial unavailability. Other lengths, head/drive families and material/grade variants require separate reviewed entries.

Additional pan, button, countersunk and hexalobular/Phillips source-profile records have their own revision and coverage in the same curation file. Their sources and engineering adaptations are described in [SCREW-PROFILE-NOTICES.md](SCREW-PROFILE-NOTICES.md). They do not revise the nineteen retained presets described above.
