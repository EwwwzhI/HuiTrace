// Isolated production-frontend benchmark. Never connects to a native backend.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../.next-verify');
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.png': 'image/png', '.ico': 'image/x-icon', '.woff2': 'font/woff2', '.txt': 'text/plain' };
http.createServer((req, res) => {
  const url = new URL(req.url, 'http://localhost');
  if (url.pathname === '/__perf.js') {
    res.setHeader('content-type', 'text/javascript');
    return res.end(fs.readFileSync(path.join(__dirname, 'performance-fixture.js')));
  }
  const relative = decodeURIComponent(url.pathname).replace(/^\/+/, '');
  let file = path.resolve(root, relative || 'index.html');
  if (!file.startsWith(root + path.sep)) { res.writeHead(403); return res.end(); }
  if (!path.extname(file)) file += '.html';
  if (!fs.existsSync(file)) { res.writeHead(404); return res.end(); }
  res.setHeader('content-type', types[path.extname(file)] || 'application/octet-stream');
  res.setHeader('cache-control', 'no-store');
  let body = fs.readFileSync(file);
  if (file.endsWith('.html')) body = body.toString().replace('<head>', '<head><script src="/__perf.js"></script>');
  res.end(body);
}).listen(3219, '127.0.0.1', () => console.log('Isolated frontend benchmark: http://127.0.0.1:3219'));
