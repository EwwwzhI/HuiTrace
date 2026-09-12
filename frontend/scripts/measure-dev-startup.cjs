// Separate dev server and cache: does not restart the user's Tauri session.
const { spawn, spawnSync } = require('node:child_process');
const { performance } = require('node:perf_hooks');
const fs = require('node:fs');
const path = require('node:path');
const label = process.argv[2] || 'before';
if (!/^[a-z0-9-]+$/.test(label)) throw new Error('Use an alphanumeric benchmark label');
fs.mkdirSync(path.resolve(__dirname, '../../output/performance'), { recursive: true });
const { warmup, STARTUP_ROUTES } = require('./tauri-dev-server');
const started = performance.now();
const log = fs.openSync(path.resolve(__dirname, `../../output/performance/dev-${label}.log`), 'w');
const child = spawn(process.execPath, [require.resolve('next/dist/bin/next'), 'dev', '-H', '127.0.0.1', '-p', '3221'], {
  cwd: path.resolve(__dirname, '..'), windowsHide: true, stdio: ['ignore', log, log],
  env: { ...process.env, NEXT_DIST_DIR: `.next-perf-${label}-${Date.now()}`, TAURI_DEV_WARMUP: '1' },
});
(async () => {
  try {
    if (label === 'before') {
      await warmup('http://127.0.0.1:3221', { routes: STARTUP_ROUTES });
      const result = { label, readyMs: performance.now() - started, routes: STARTUP_ROUTES };
      fs.writeFileSync(path.resolve(__dirname, `../../output/performance/dev-${label}.json`), JSON.stringify(result, null, 2));
      console.log(result);
    } else {
      await warmup('http://127.0.0.1:3221', { routes: ['/'] });
      const readyMs = performance.now() - started;
      await warmup('http://127.0.0.1:3221', { routes: STARTUP_ROUTES.filter(route => route !== '/') });
      const result = { label, readyMs, allRoutesMs: performance.now() - started, routes: STARTUP_ROUTES };
      fs.writeFileSync(path.resolve(__dirname, `../../output/performance/dev-${label}.json`), JSON.stringify(result, null, 2));
      console.log(result);
    }
  } finally {
    if (process.platform === 'win32') spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true });
    else child.kill('SIGTERM');
    fs.closeSync(log);
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
