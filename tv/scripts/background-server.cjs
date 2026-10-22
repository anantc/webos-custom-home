// Preview-only photo origin. Production uses the published GitHub assets.
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const root = path.resolve(__dirname, '../dist/backgrounds-v1');
http.createServer((req, res) => {
  const name = new URL(req.url, 'http://localhost').pathname.slice(1);
  if (!/^[a-zA-Z0-9_.-]+$/.test(name)) { res.writeHead(404).end(); return; }
  const file = path.join(root, name);
  if (!fs.existsSync(file)) { res.writeHead(404).end(); return; }
  res.setHeader('Content-Type', name.endsWith('.jpg') ? 'image/jpeg' : 'application/json');
  fs.createReadStream(file).pipe(res);
}).listen(8793, '127.0.0.1', () => console.log('Photo preview origin: http://127.0.0.1:8793/'));
