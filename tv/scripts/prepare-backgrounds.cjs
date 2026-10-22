// Prepare the reviewed photos as independently hosted, content-addressed release assets.
const fs = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '..');
const digest = bytes => crypto.createHash('sha256').update(bytes).digest('hex');

async function main() {
  const update = process.argv.includes('--update');
  const out = path.join(root, 'dist/backgrounds-v1');
  await fs.mkdir(out, { recursive: true });
  const selection = JSON.parse(await fs.readFile(path.join(root, 'backgrounds/selection.json')));
  const holidays = JSON.parse(await fs.readFile(path.join(root, 'backgrounds/holidays.json')));
  const manifestPath = path.join(root, 'backgrounds/manifest.json');
  const previous = await fs.readFile(manifestPath).then(JSON.parse).catch(() => null);
  if (!previous && !update) throw Error('Run with --update to create the initial manifest.');
  const photos = [];
  // Sequential downloads keep source traffic modest and make failures easy to identify.
  for (const photo of selection) {
    const old = previous?.photos.find(p => p.id === photo.id);
    let bytes = old && await fs.readFile(path.join(out, old.file)).catch(() => null);
    if (old && (!bytes || digest(bytes) !== old.sha256)) {
      // Metadata-only publications reuse the reviewed JPEG, even if Unsplash's
      // image processing has since changed. New photos fall back to their source.
      try {
        const response = await fetch(old.url, { signal: AbortSignal.timeout(45000) });
        if (!response.ok) throw Error(`HTTP ${response.status}`);
        const hosted = Buffer.from(await response.arrayBuffer());
        if (digest(hosted) !== old.sha256) throw Error('hosted checksum mismatch');
        bytes = hosted;
      } catch (error) {
        console.warn(`${photo.id}: hosted copy unavailable (${error.message}); checking Unsplash source`);
      }
    }
    if (!bytes || digest(bytes) !== old.sha256) {
      const url = new URL(photo.image);
      url.search = new URLSearchParams({ fm: 'jpg', fit: 'crop', crop: 'center', w: '1920', h: '1080', q: '85' });
      const response = await fetch(url, { signal: AbortSignal.timeout(45000) });
      if (!response.ok) throw Error(`${photo.id}: HTTP ${response.status}`);
      bytes = Buffer.from(await response.arrayBuffer());
    }
    if (bytes.length > 4 * 1024 * 1024 || bytes[0] !== 255 || bytes[1] !== 216) throw Error(`${photo.id}: invalid JPEG`);
    const sha256 = digest(bytes);
    if (old && old.sha256 !== sha256 && !update) throw Error(`${photo.id}: source changed; review before --update`);
    const file = `${photo.id}-${sha256.slice(0, 16)}.jpg`;
    await fs.writeFile(path.join(out, file), bytes);
    const { image, ...credit } = photo;
    photos.push({ ...credit, file, sha256, bytes: bytes.length,
      url: `https://github.com/anantc/webos-custom-home/releases/download/backgrounds-v1/${file}` });
    console.log(`${photo.id} ${Math.round(bytes.length / 1024)} KiB`);
  }
  const manifest = { version: 2, revision: update ? Math.max(1, (previous?.revision || 0) + 1) : previous.revision, holidays, photos };
  const json = JSON.stringify(manifest, null, 2) + '\n';
  await fs.writeFile(path.join(out, 'manifest.json'), json);
  if (update) await fs.writeFile(manifestPath, json);
  const credits = photos.map(p => `${p.id} — ${p.title}\n${p.caption ? `Caption: ${p.caption}\n` : ''}Photo by ${p.author} on Unsplash\n${p.source}\nLicense: ${p.license}\n`).join('\n');
  await fs.writeFile(path.join(out, 'CREDITS.txt'), credits);
  await fs.writeFile(path.join(out, 'SHA256SUMS'), photos.map(p => `${p.sha256}  ${p.file}\n`).join(''));
  console.log(`Prepared ${photos.length} photos in ${out}`);
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
