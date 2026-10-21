// noVNC static files and its WebSocket-to-VNC bridge, using Node's packaged ws module.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const net = require('node:net');
const { WebSocketServer } = require('ws');
const root = '/usr/share/novnc';
const types = { '.html': 'text/html', '.js': 'application/javascript', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png', '.json': 'application/json' };
const server = http.createServer((request, response) => {
  let file;
  try {
    const url = new URL(request.url, 'http://localhost');
    file = path.resolve(root, '.' + decodeURIComponent(url.pathname === '/' ? '/vnc.html' : url.pathname));
    if (!file.startsWith(root + '/') || !fs.statSync(file).isFile()) throw new Error('not found');
  } catch { response.writeHead(404); response.end(); return; }
  response.setHeader('Content-Type', types[path.extname(file)] || 'application/octet-stream');
  const stream = fs.createReadStream(file);
  stream.on('error', () => response.destroy()); stream.pipe(response);
});
const sockets = new WebSocketServer({ server, maxPayload: 8 * 1024 * 1024, handleProtocols: protocols => protocols.has('binary') ? 'binary' : false });
sockets.on('connection', ws => {
  const target = net.connect({ host: '127.0.0.1', port: 5900 });
  const close = () => { target.destroy(); ws.terminate(); };
  target.on('data', bytes => {
    target.pause();
    if (ws.readyState !== ws.OPEN) return close();
    ws.send(bytes, { binary: true }, error => error ? close() : target.resume());
  });
  ws.on('message', (bytes, binary) => {
    if (!binary) return close();
    if (!target.write(bytes)) ws.pause();
  });
  target.on('drain', () => ws.resume());
  target.on('error', close); target.on('close', close);
  ws.on('error', close); ws.on('close', () => target.destroy());
});
server.listen(6080, '0.0.0.0'); // Docker publishes this only on the host's 127.0.0.1.
