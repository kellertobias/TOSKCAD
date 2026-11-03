# Washer dimensional data notices

The washer dimensional dataset is adapted from `plainwasher1` in the accompanying unmodified `washer.blt`. The original file identifies Johannes Reinhardt, copyright 2013, and grants the GNU Lesser General Public License, version 2.1 or any later version. The license and warranty disclaimer remain in that source file; the complete license is in [COPYING.LESSER](COPYING.LESSER).

The exact upstream commit, source URL and SHA256 are recorded in [washers-curation.json](../../washers-curation.json). Its adapted dimensional data and the corresponding dimensional data retained in the generated washer projects are provided under LGPL-2.1-or-later. This notice, the original source, the adapted curation source and the license accompany the bundled Library. There is no warranty of merchantability or fitness for a particular purpose.

## Adaptation notice, 2026-11-03

MultiCAD's catalog maintainers selected the ten supported normal-series rows identified in the curation file, retained their explicit fastener nominal separately from physical bore, and corrected the M10 bore from the upstream 10.0 mm to 10.5 mm. That correction follows the [NBK JIS B 1256:2008 excerpt](https://www.nbk1560.com/images/en-US/product/technical_data/hira_zagane_NBK/hira_zagane_NBK_1.pdf), PDF page 2, normal series product grade A. Outside diameter and thickness are unchanged for that row. The original `washer.blt` is not modified.

The same NBK table corroborates the other selected rows except M7. Its product grade A refers to the source's dimensional table. The M7 triple is independently corroborated by [RECA article 54077, EAN 4029484178117](https://shop.reca.fr/Washer-DIN-125-200-HV-zinc-plated/54077.sku/en/US/EUR/), whose page identifies the screw nominal and physical dimensions separately. No dated revision was identified for that product page. Manufacturer material, coating and hardness descriptions are not transferred into the generic dataset.

The upstream table lists legacy DIN 125 A and ISO 7089 labels without editions. This adaptation does not establish compliance with a current edition. Its precise series identity is BOLTS `plainwasher1` normal-series reference dimensions at the recorded commit, with the declared correction. NBK's small-series table on PDF page 1 is a separate series and is not substituted for these normal-series dimensions.

The native washer projects contain independently authored annular solids, editable physical dimensions, retained literal reference dimensions and software-rendered native previews. They omit dimensional tolerances, edge breaks and chamfers. Material and grade are unspecified. Their reference metadata does not establish stock availability, manufactured fit or a supplier's product certification. No manufacturer CAD, meshes, illustrations, logos or catalog prose are redistributed.

The complete requested nominal coverage, including explicit unavailable/custom cases, lives in the curation file. An unavailable row means that this selected source series supplies no preset; it does not state that no commercial washer of that nominal exists.
