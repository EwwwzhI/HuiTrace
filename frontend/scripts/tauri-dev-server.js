// Keep Tauri's devUrl closed until Next has compiled and served the initial scripts.
// A TCP relay preserves HTTP and WebSocket/HMR traffic without rewriting responses.
const net = require('node:net');
const { spawn, spawnSync } = require('node:child_process');
const path = require('node:path');
const { setTimeout: delay } = require('node:timers/promises');
const STARTUP_ROUTES = ['/', '/meeting-details', '/settings', '/actions'];

async function prepareFrontend(origin, onReady, {
  warm = warmup,
  onBackgroundError = error => console.warn('[tauri-dev] Optional page warmup failed; Next will retry on navigation:', error.message),
} = {}) {
  // The home page and its complete scripts are the only startup dependency.
  await warm(origin, { routes: ['/'] });
  await onReady();
  try {
    await warm(origin, { routes: STARTUP_ROUTES.filter(route => route !== '/') });
  } catch (error) {
    // An optional route failure must not close an already usable application.
    onBackgroundError(error);
  }
}

async function warmup(origin, { timeoutMs = 120000, retryMs = 500, routes = ['/'] } = {}) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  let successfulPasses = 0;
  while (Date.now() < deadline) {
    try {
      const read = async (url) => {
        const response = await fetch(url, {
          signal: AbortSignal.timeout(Math.max(1, Math.min(20000, deadline - Date.now()))),
          headers: { 'accept-encoding': 'identity' },
        });
        if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
        // Reading the whole body detects truncated chunks, unlike a HEAD request.
        return response.text();
      };
      const scriptUrls = new Set();
      for (const route of routes) {
        const html = await read(`${origin}${route}`);
        const scripts = [...html.matchAll(/<script\b[^>]*\bsrc="([^"]+)"/g)]
          .map(match => new URL(match[1].replaceAll('&amp;', '&'), origin))
          .filter(url => url.origin === origin && url.pathname.startsWith('/_next/'));
        if (!scripts.length) throw new Error(`No Next.js scripts found on ${route}`);
        for (const url of scripts) scriptUrls.add(url.href);
      }
      for (const url of scriptUrls) await read(url);
      if (++successfulPasses >= 2) return;
    } catch (error) {
      successfulPasses = 0;
      lastError = error;
    }
    await delay(retryMs);
  }
  throw new Error(`Frontend warmup timed out: ${lastError?.message || 'scripts did not stabilize'}`);
}

function createRelay(port) {
  return net.createServer(client => {
    const upstream = net.connect(port, '127.0.0.1');
    client.on('error', () => upstream.destroy());
    upstream.on('error', () => client.destroy());
    client.on('close', () => upstream.destroy());
    upstream.on('close', () => client.destroy());
    client.pipe(upstream).pipe(client);
  });
}

async function main() {
  const publicPort = 3118;
  const occupied = await new Promise((resolve, reject) => {
    const socket = net.connect(publicPort, 'localhost');
    socket.once('connect', () => { socket.destroy(); resolve(true); });
    socket.once('error', error => error.code === 'ECONNREFUSED' ? resolve(false) : reject(error));
  });
  if (occupied) throw new Error('Port 3118 is already in use. Stop the existing dev server before starting Tauri.');
  // Allocate an internal loopback port so this does not require a second fixed port.
  const reservation = net.createServer();
  await new Promise((resolve, reject) => {
    reservation.once('error', reject);
    reservation.listen(0, '127.0.0.1', resolve);
  });
  const internalPort = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  const child = spawn(process.execPath, [
    require.resolve('next/dist/bin/next'), 'dev', '-H', '127.0.0.1', '-p', String(internalPort),
  ], {
    cwd: path.resolve(__dirname, '..'), stdio: 'inherit', windowsHide: true,
    env: { ...process.env, TAURI_DEV_WARMUP: '1' },
  });
  const relay = createRelay(internalPort);
  let stopping = false;
  function stop(code) {
    if (stopping) return;
    stopping = true;
    relay.close();
    if (child.pid && child.exitCode === null) {
      if (process.platform === 'win32') {
        spawnSync('taskkill', ['/pid', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
      } else child.kill('SIGTERM');
    }
    process.exit(code);
  }
  process.on('SIGINT', () => stop(0));
  process.on('SIGTERM', () => stop(0));
  child.on('error', error => { console.error(error.message); stop(1); });
  child.on('exit', code => stop(code ?? 1));
  relay.on('error', error => { console.error(`[tauri-dev] ${error.message}`); stop(1); });
  try {
    const started = performance.now();
    console.log('[tauri-dev] Compiling home and checking complete script downloads...');
    await prepareFrontend(`http://127.0.0.1:${internalPort}`, () => new Promise(resolve => {
      relay.listen(publicPort, () => {
        console.log(`[tauri-dev] Frontend ready in ${Math.round(performance.now() - started)}ms: http://localhost:${publicPort}; warming other pages in background`);
        resolve();
      });
    }));
  } catch (error) {
    console.error(`[tauri-dev] ${error.message}`);
    stop(1);
  }
}

if (require.main === module) main().catch(error => { console.error(error); process.exit(1); });
module.exports = { warmup, createRelay, STARTUP_ROUTES, prepareFrontend };
