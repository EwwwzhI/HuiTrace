/* Run with Node and sharp available (Next.js also installs sharp).
 * node design/huitrace/yin-wave-v2/build-icons.cjs [--apply]
 * Keep the 256px ICO entry FIRST: Tauri codegen decodes entries()[0].
 */
const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
const { execFileSync } = require('node:child_process');
const root = path.resolve(__dirname, '../../..');
let sharp;
try { sharp = require('sharp'); }
catch { sharp = createRequire(fs.realpathSync(path.join(root, 'frontend/node_modules/next/package.json')))('sharp'); }
const source = fs.readFileSync(path.join(__dirname, 'icon.svg'), 'utf8');
const sizes = [256, 128, 96, 64, 48, 40, 32, 24, 20, 16];

// At small sizes, align the voices to physical pixels; antialias the curves
// by rendering at 4x and reducing once. Never overwrite individual pixels.
function svgAt(size) {
  if (size > 48) return source;
  const scale = size / 64;
  const width = Math.max(1, Math.round(size * 3 / 64));
  const gap = Math.max(1, Math.round(size * 2 / 64));
  const total = width * 3 + gap * 2;
  const groups = [[41.32, 20.89, '#fcfbf7', [6, 13, 8]], [22.68, 43.11, '#141619', [8, 13, 6]]];
  const voices = groups.map(([cx, cy, fill, heights]) => {
    const left = Math.round(cx * scale - total / 2);
    return heights.map((h, i) => {
      const height = Math.max(width + 1, Math.round(h * scale));
      const x = (left + i * (width + gap)) / scale;
      const y = Math.round(cy * scale - height / 2) / scale;
      return `<rect x="${x}" y="${y}" width="${width / scale}" height="${height / scale}" rx="${width / scale / 2}" fill="${fill}"/>`;
    }).join('');
  }).join('');
  return source.replace(/<g id="voices">[\s\S]*?<\/g>\s*<\/g>/, `<g id="voices">${voices}</g>`);
}

async function render(svg, size) {
  return sharp(Buffer.from(svg), { density: 72 * size * 4 / 1024 })
    .resize(size, size, { kernel: 'lanczos3' }).ensureAlpha().png().toBuffer();
}

async function main() {
  const frames = [];
  for (const size of sizes) {
    const png = await render(svgAt(size), size);
    fs.writeFileSync(path.join(__dirname, `icon-${size}.png`), png);
    frames.push(png);
  }
  const header = Buffer.alloc(6 + sizes.length * 16);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(sizes.length, 4);
  let offset = header.length;
  sizes.forEach((size, i) => {
    const p = 6 + i * 16;
    header[p] = header[p + 1] = size === 256 ? 0 : size;
    header.writeUInt16LE(1, p + 4);
    header.writeUInt16LE(32, p + 6);
    header.writeUInt32LE(frames[i].length, p + 8);
    header.writeUInt32LE(offset, p + 12);
    offset += frames[i].length;
  });
  fs.writeFileSync(path.join(__dirname, 'huitrace-yin-wave.ico'), Buffer.concat([header, ...frames]));
  fs.writeFileSync(path.join(__dirname, 'huitrace-yin-wave.png'), await render(source, 1024));
  fs.writeFileSync(path.join(__dirname, 'icon-small.svg'), svgAt(24));

  if (process.argv.includes('--apply')) {
    const frontend = path.join(root, 'frontend');
    const cli = path.join(frontend, 'node_modules/@tauri-apps/cli/tauri.js');
    execFileSync(process.execPath, [cli, 'icon', path.join(__dirname, 'icon.svg'), '--output', path.join(frontend, 'src-tauri/icons')], { stdio: 'inherit' });
    const copy = (from, to) => fs.copyFileSync(path.join(__dirname, from), path.join(frontend, to));
    copy('huitrace-yin-wave.ico', 'src-tauri/icons/icon.ico');
    copy('huitrace-yin-wave.ico', 'src/app/favicon.ico');
    for (const size of [32, 64, 128]) copy(`icon-${size}.png`, `src-tauri/icons/${size}x${size}.png`);
    copy('icon-256.png', 'src-tauri/icons/128x128@2x.png');
    copy('icon.svg', 'public/huitrace-icon-yin-wave.svg');
    copy('icon-small.svg', 'public/huitrace-icon-yin-wave-small.svg');
    copy('huitrace-yin-wave.png', 'public/huitrace-icon-yin-wave.png');
    copy('huitrace-yin-wave.png', 'public/huitrace-icon.png');
    copy('icon-128.png', 'public/icon_128x128.png');
    copy('icon-64.png', 'public/icon_32x32@2x.png');
  }
  console.log(`Exported ${sizes.length} antialiased ICO frames; first frame: ${sizes[0]}px.`);
}
main().catch(error => { console.error(error); process.exitCode = 1; });
