const { test } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const net = require('node:net');
const { warmup, createRelay, prepareFrontend } = require('./tauri-dev-server');

test('opens the frontend after home, without waiting for optional pages', async () => {
  const order = [];
  let finishBackground;
  const pending = prepareFrontend('http://fixture', () => { order.push('ready'); }, {
    warm: async (_origin, { routes }) => {
      order.push(routes.join(','));
      if (routes[0] !== '/') await new Promise(resolve => { finishBackground = resolve; });
    },
  });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(order, ['/', 'ready', '/meeting-details,/settings,/actions']);
  finishBackground();
  await pending;
});

test('optional page failure leaves the frontend open, but home failure blocks it', async () => {
  const events = [];
  await prepareFrontend('http://fixture', () => events.push('ready'), {
    warm: async (_origin, { routes }) => { if (routes[0] !== '/') throw new Error('optional compilation'); },
    onBackgroundError: error => events.push(error.message),
  });
  assert.deepEqual(events, ['ready', 'optional compilation']);
  await assert.rejects(prepareFrontend('http://fixture', () => events.push('unexpected ready'), {
    warm: async () => { throw new Error('home compilation'); },
  }), /home compilation/);
  assert.equal(events.includes('unexpected ready'), false);
});

async function listen(server) {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return server.address().port;
}
async function close(server) {
  server.closeAllConnections?.();
  await new Promise(resolve => server.close(resolve));
}

test('warms meeting scripts too and deduplicates shared layout scripts per pass', async () => {
  const counts = new Map();
  const server = http.createServer((req, res) => {
    counts.set(req.url, (counts.get(req.url) || 0) + 1);
    if (req.url === '/' || req.url === '/meeting-details') {
      return res.end('<script src="/_next/layout.js"></script>' +
        (req.url === '/meeting-details' ? '<script src="/_next/meeting.js"></script>' : ''));
    }
    res.end('script');
  });
  const port = await listen(server);
  try {
    await warmup(`http://127.0.0.1:${port}`, { routes: ['/', '/meeting-details'], retryMs: 1 });
    assert.equal(counts.get('/meeting-details'), 2);
    assert.equal(counts.get('/_next/meeting.js'), 2);
    assert.equal(counts.get('/_next/layout.js'), 2);
  } finally { await close(server); }
});

test('retries a truncated script and requires two complete passes', async () => {
  let chunks = 0;
  const server = http.createServer((req, res) => {
    if (req.url === '/') return res.end('<script src="/_next/static/chunks/app/layout.js"></script>');
    chunks++;
    if (chunks === 1) {
      res.writeHead(200, { 'content-length': 100 });
      res.write('partial');
      setImmediate(() => res.destroy());
    } else res.end('complete script');
  });
  const port = await listen(server);
  try {
    await warmup(`http://127.0.0.1:${port}`, { timeoutMs: 3000, retryMs: 1 });
    assert.equal(chunks, 3);
  } finally { await close(server); }
});

test('fails rather than declaring a compilation error page ready', async () => {
  const server = http.createServer((req, res) => { res.writeHead(500); res.end('Compile error'); });
  const port = await listen(server);
  try {
    await assert.rejects(warmup(`http://127.0.0.1:${port}`, { timeoutMs: 100, retryMs: 1 }), /warmup timed out/);
  } finally { await close(server); }
});

test('relay preserves HTTP upgrade traffic for HMR', async () => {
  const upstream = http.createServer();
  upstream.on('upgrade', (req, socket) => {
    socket.end('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\nhmr-payload');
  });
  const upstreamPort = await listen(upstream);
  const relay = createRelay(upstreamPort);
  const port = await listen(relay);
  try {
    const result = await new Promise((resolve, reject) => {
      const client = net.connect(port, '127.0.0.1', () => {
        client.write('GET /_next/webpack-hmr HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n');
      });
      let data = '';
      client.on('data', chunk => { data += chunk; });
      client.on('end', () => resolve(data));
      client.on('error', reject);
    });
    assert.match(result, /101 Switching Protocols/);
    assert.match(result, /hmr-payload/);
  } finally { await close(relay); await close(upstream); }
});
