import assert from 'node:assert/strict';
import test from 'node:test';
import { downloadsFor, publishedReleases } from '../website/releases.mjs';
const base = 'https://github.com/kellertobias/TOSKCAD';
function fixture(tag = 'v1.2.3') {
  const names = ['macos-arm64', 'windows-x64', 'linux-x64'].flatMap(target => {
    const stem = `multicad-${tag.slice(1)}-${target}`;
    return [`${stem}.zip`, `${stem}.zip.sha256`, `${stem}-sources.zip`];
  });
  return { tag_name: tag, html_url: `${base}/releases/tag/${tag}`, published_at: '2026-10-31T12:00:00Z',
    assets: names.map(name => ({ name, size: 1048576, browser_download_url: `${base}/releases/download/${tag}/${name}` })) };
}
test('release page selects exact runtime, source and checksum assets for every platform', () => {
  const downloads = downloadsFor(fixture());
  assert.equal(downloads.length, 3);
  for (const entry of downloads) {
    assert(entry.binary.name.endsWith(`${entry.target}.zip`));
    assert(entry.checksum.name.endsWith(`${entry.target}.zip.sha256`));
    assert(entry.source.name.endsWith(`${entry.target}-sources.zip`));
  }
});
test('incomplete, empty or foreign download assets cannot become download links', () => {
  const release = fixture();
  release.assets[0].browser_download_url = 'https://other.example/app.zip';
  release.assets[3].size = 0;
  release.assets.splice(6, 3);
  const downloads = downloadsFor(release);
  assert.equal(downloads[0].binary, null);
  assert.equal(downloads[1].binary, null);
  assert.equal(downloads[2].binary, null);
  assert.deepEqual(downloadsFor({ tag_name: '../bad' }), []);
});
test('only published releases from this repository are shown, newest first', () => {
  const current = fixture();
  const old = { ...fixture('v1.2.2'), published_at: '2026-10-01T12:00:00Z' };
  assert.deepEqual(publishedReleases([old, { ...current, draft: true }, current,
    { ...current, html_url: 'javascript:alert(1)' }, { ...current, published_at: 'bad' }]), [current, old]);
  assert.deepEqual(publishedReleases([]), []);
  assert.throws(() => publishedReleases({ message: 'rate limited' }));
});
