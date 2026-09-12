const fs = require('node:fs');
const path = require('node:path');
const { createRequire } = require('node:module');
let sharp;
try { sharp = require('sharp'); }
catch { sharp = createRequire(fs.realpathSync(path.resolve(__dirname, '../../../frontend/node_modules/next/package.json')))('sharp'); }
const dir = __dirname;
async function main() {
  const overlays = [];
  const text = [];
  const label = (x, y, value, fill = '#71716b', size = 15) => text.push(`<text x="${x}" y="${y}" fill="${fill}" font-family="Segoe UI, sans-serif" font-size="${size}">${value}</text>`);
  label(40, 48, 'HuiTrace / Yin-wave', '#252722', 26);
  label(40, 80, 'A dialogue in balance. Two voices, one continuous flow.');
  for (const [file, x, title] of [['../yin-wave-v1/icon-256.png', 55, 'Before'], ['icon-256.png', 365, 'Refined']]) {
    overlays.push({ input: await sharp(path.join(dir, file)).resize(200, 200).toBuffer(), left: x, top: 120 });
    label(x, 350, title, '#252722', 18);
  }
  label(680, 136, 'Actual pixel sizes', '#252722', 18);
  for (const [i, size] of [16, 20, 24, 32, 40, 48].entries()) {
    const x = 680 + i * 53;
    overlays.push({ input: path.join(dir, `icon-${size}.png`), left: x, top: 176 + Math.floor((48 - size) / 2) });
    label(x, 248, `${size}`);
    overlays.push({ input: path.join(dir, `icon-${size}.png`), left: x, top: 290 + Math.floor((48 - size) / 2) });
  }
  label(40, 414, 'Taskbar detail / 32 px enlarged 5x', '#252722', 18);
  for (const [file, x] of [['../yin-wave-v1/icon-32.png', 60], ['icon-32.png', 370]]) {
    overlays.push({ input: await sharp(path.join(dir, file)).resize(160, 160, { kernel: 'nearest' }).toBuffer(), left: x, top: 440 });
  }
  label(680, 440, 'Continuous S curve');
  label(680, 474, 'Two opposite-color waveforms');
  label(680, 508, 'Antialiased, pixel-aligned small sizes');
  label(680, 542, '256 px first frame for Tauri');
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1040" height="640"><rect width="1040" height="640" fill="#f4f3ee"/><rect x="658" y="270" width="342" height="86" rx="12" fill="#232528"/>${text.join('')}</svg>`;
  await sharp(Buffer.from(svg)).composite(overlays).png().toFile(path.join(dir, 'preview.png'));
}
main().catch(e => { console.error(e); process.exitCode = 1; });
