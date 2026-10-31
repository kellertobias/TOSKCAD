const repository = 'https://github.com/kellertobias/TOSKCAD';
export const platforms = [
  { target: 'macos-arm64', name: 'macOS', detail: 'Apple silicon · macOS 13 or newer' },
  { target: 'windows-x64', name: 'Windows', detail: '64-bit Intel / AMD' },
  { target: 'linux-x64', name: 'Linux', detail: '64-bit Intel / AMD · Ubuntu 24.04 or newer' },
];
const versionPattern = /^v(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/;

// Accept only GitHub-hosted assets belonging to this exact release.
export function releaseAsset(release, name) {
  const asset = release.assets?.find(asset => asset.name === name);
  if (!asset || !Number.isSafeInteger(asset.size) || asset.size <= 0) return null;
  const expected = `${repository}/releases/download/${encodeURIComponent(release.tag_name)}/${encodeURIComponent(name)}`;
  return asset.browser_download_url === expected ? asset : null;
}

export function publishedReleases(data) {
  if (!Array.isArray(data)) throw new Error('Unexpected GitHub response');
  return data.filter(release => !release.draft && versionPattern.test(release.tag_name) &&
    release.html_url === `${repository}/releases/tag/${encodeURIComponent(release.tag_name)}` &&
    Number.isFinite(Date.parse(release.published_at)) && Array.isArray(release.assets))
    .sort((a, b) => Date.parse(b.published_at) - Date.parse(a.published_at)).slice(0, 6);
}

export function downloadsFor(release) {
  const version = release.tag_name.match(versionPattern)?.[1];
  if (!version) return [];
  return platforms.map(platform => {
    const name = `multicad-${version}-${platform.target}.zip`;
    return { ...platform, binary: releaseAsset(release, name), checksum: releaseAsset(release, `${name}.sha256`),
      source: releaseAsset(release, name.replace('.zip', '-sources.zip')) };
  });
}

function element(tag, text, className) {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  if (className) node.className = className;
  return node;
}
function link(text, href, className) {
  const node = element('a', text, className);
  node.href = href;
  return node;
}
function renderRelease(release, index) {
  const section = element('section', null, 'release-entry');
  const heading = element('div', null, 'release-heading');
  const title = element('h2', release.tag_name);
  title.id = `release-${index}`;
  section.setAttribute('aria-labelledby', title.id);
  const date = element('time', new Intl.DateTimeFormat('en', { dateStyle: 'long' }).format(new Date(release.published_at)));
  date.dateTime = release.published_at;
  heading.append(title, date, element('p', release.prerelease ? 'Pre-release development build' : 'Development build'), link('Release notes on GitHub ↗', release.html_url));
  const downloads = element('div', null, 'download-list');
  for (const platform of downloadsFor(release)) {
    const row = element('div', null, 'download-row');
    const label = element('div');
    label.append(element('h3', platform.name), element('p', platform.detail));
    const files = element('div', null, 'download-files');
    if (platform.binary && platform.checksum && platform.source) {
      const size = (platform.binary.size / 1024 / 1024).toLocaleString('en', { maximumFractionDigits: 1 });
      files.append(link(`Download ${platform.name} · ${size} MB`, platform.binary.browser_download_url, 'action'));
      const supporting = element('div', null, 'download-support');
      supporting.append(link(`${platform.name} SHA-256`, platform.checksum.browser_download_url), link(`${platform.name} sources`, platform.source.browser_download_url));
      files.append(supporting);
    } else {
      files.append(element('p', 'A complete download is not available for this platform in this release.'));
    }
    row.append(label, files);
    downloads.append(row);
  }
  section.append(heading, downloads);
  return section;
}

export async function loadReleases() {
  const status = document.querySelector('#release-status');
  const title = document.querySelector('#status-title');
  const copy = document.querySelector('#status-copy');
  const retry = document.querySelector('#retry-releases');
  const list = document.querySelector('#releases');
  retry.hidden = true;
  status.hidden = false;
  title.textContent = 'Finding the latest release…';
  copy.textContent = 'Checking GitHub for published downloads.';
  list.setAttribute('aria-busy', 'true');
  try {
    const response = await fetch('https://api.github.com/repos/kellertobias/TOSKCAD/releases?per_page=20', {
      headers: { Accept: 'application/vnd.github+json' }, signal: AbortSignal.timeout(15000),
    });
    if (!response.ok) throw new Error(`GitHub returned ${response.status}`);
    const releases = publishedReleases(await response.json());
    list.replaceChildren(...releases.map(renderRelease));
    if (releases.length) status.hidden = true;
    else {
      title.textContent = 'No downloads published yet.';
      copy.textContent = 'The first release will appear here after its platform builds pass. You can follow progress on GitHub or build from source.';
    }
  } catch {
    title.textContent = 'Couldn’t load the release list.';
    copy.textContent = 'GitHub may be temporarily unavailable or limiting requests. Try again, or use the GitHub Releases link above to download directly.';
    retry.hidden = false;
  } finally {
    list.setAttribute('aria-busy', 'false');
  }
}
if (typeof document !== 'undefined') {
  document.querySelector('#retry-releases').addEventListener('click', loadReleases);
  loadReleases();
}
