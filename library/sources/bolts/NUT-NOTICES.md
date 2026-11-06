# Nut dimensional data notices

The nut dimensional dataset is adapted from `hexagonnut2` and `hexagonnut1` in the accompanying unmodified [nut.blt](nut.blt). The original file identifies Johannes Reinhardt, copyright 2013, and grants the GNU Lesser General Public License, version 2.1 or any later version. Its attribution and warranty disclaimer remain intact; the complete license is in [COPYING.LESSER](COPYING.LESSER).

The exact upstream commit, source URL and SHA256 are recorded in [nuts-curation.json](../../nuts-curation.json). Its adapted dimensional data and corresponding dimensional data retained in generated nut projects are provided under LGPL-2.1-or-later. This notice, the original source, the adapted curation source and the license accompany the dataset. There is no warranty of merchantability or fitness for a particular purpose.

## Adaptation notice, 2026-11-06

MultiCAD's catalog maintainers selected nineteen discrete metric nut rows: ten DIN 934:1987 references and nine ISO 4032:2012 references. The selected BOLTS rows are numerically unchanged. The adaptation supplies independently checked coarse pitches, separate edition identities, primary cross-check references and twenty-four per-family coverage statuses. BOLTS does not itself name a DIN or ISO edition, and its nut table contains no pitch field.

The across-flats reference uses the published `s` nominal/max value; height uses `m` max. These are chosen modeling references, not midpoint targets or a complete tolerance model. `source_row` retains BOLTS `[d1, s, m_max, e_min]` in that order. Here `d1` means the nominal thread designation, not a plain cylindrical hole. An ideal sharp regular hexagon derives its corner diameter from across flats. The retained minimum corner dimension is provenance and a cross-check, not a second imposed ideal-hex dimension.

## Edition and source boundaries

The [DIN publisher record](https://www.dinmedia.de/de/norm/din-934/3074472) identifies DIN 934:1987-10 as withdrawn. The [BSN table](https://bsnfastener.com/pdf/Section04/catalog_P4-11_DIN_Nut_Steel.pdf), catalog page 4-11, explicitly names DIN 934:1987 and verifies the selected M5/M6/M8/M10 limits and coarse pitches. No catalog publication date was identified. The [Böllhoff DIN 934 sheet](https://eshop.boellhoff.de/out/media/pdf/DIN_934_Edelstahl_A2___en.pdf), page 1, cross-checks the smaller selected rows; it specifies no edition or publication date. These manufacturer references do not constitute direct inspection of the complete DIN standard.

The DIN M7 coarse-pitch preset is independently corroborated by [Walraven BIS Hexagon Nut K45 35](https://library.walraven.com/repository/rnd/documents/EN-INT/BIS%20Hexagon%20Nut%20-%20K%2045%2035%20%28EN-INT%29.pdf), page 1, EN-INT 10/2024, part 6123007 M7B. Its P1/AF11/height5.5 values match the selected references. That sheet does not specify `e_min` or a DIN edition; the curation retains BOLTS `e_min` without inventing independent verification of that field. The distinct M7F fine-pitch product is not selected.

The [ISO publisher record](https://www.iso.org/es/contents/data/standard/06/16/61668.html) and [ISO-authored public preview](https://cdn.standards.iteh.ai/samples/61668/94af161ed14040e09a945fb5b094ae16/ISO-4032-2012.pdf) identify ISO 4032:2012, fourth edition 2012-12-15. Table 1, PDF page 8 / printed page 2, verifies the selected preferred rows. Table 2, PDF page 9 / printed page 3, verifies nonpreferred M3.5. This edition is superseded, and no current-edition compliance is asserted.

ISO 4032:2012 has no M7 row. The edition-unspecified BOLTS ISO-labeled M7 is deliberately unavailable in this family. ISO 4032:2023 adds M7 with a different height and is not substituted. M4.5 and M9 have no selected row in either family. An unavailable status describes this curated matrix, not universal commercial availability.

The families remain dimensionally distinct. At M10, DIN references are AF17/height8 mm; ISO2012 references are AF16/height8.4 mm. Heights also differ at M5/M6/M8. No DIN/ISO hybrid row or silently upgraded edition is supplied.

## Modeling and rights limits

The dataset specifies a sharp hexagonal blank and the shared JIS B 0205-2/4:2001 nominal-basic internal thread convention. Modeled and cosmetic representation must remain explicit; cosmetic geometry is not a manufacturing thread. This dimensional dataset does not itself prove native geometry, manufacturing export or physical fit.

Dimensional tolerances, external/end chamfers, washer-face details and thread-entry mouth geometry are omitted. Material, grade, coating and stock are unspecified. No ISO 965 fit class, current-standard compliance, load rating, gage fit, torque capacity or print-success certification is transferred from a source product.

The licensed BOLTS data is adapted with these notices. Manufacturer material, strength and coating descriptions are not model properties. No manufacturer CAD, meshes, diagrams, logos, catalog prose or standards PDFs are redistributed, and the BOLTS license grants no rights to those separate works.
