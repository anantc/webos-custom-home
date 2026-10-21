// Run inside the build image: uses its noVNC assets and ws module, never the TV.
const test = require('node:test');
const assert = require('node:assert/strict');
const net = require('node:net');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { setTimeout: delay } = require('node:timers/promises');
const WebSocket = require('ws');

test('preview serves noVNC and bridges binary traffic in both directions', { timeout: 15000 }, async t => {
  const connections = new Set();
  const vnc = net.createServer(socket => {
    connections.add(socket);
    socket.on('close', () => connections.delete(socket));
    socket.write('RFB 003.008\n');
    socket.on('data', bytes => socket.write(bytes));
  });
  vnc.listen(5900, '127.0.0.1');
  await once(vnc, 'listening');
  const child = spawn(process.execPath, [path.join(__dirname, 'preview-server.cjs')], { stdio: ['ignore', 'ignore', 'pipe'] });
  let errors = '';
  child.stderr.on('data', bytes => { errors += bytes; });
  let ws;
  t.after(async () => {
    if (ws) ws.terminate();
    if (child.exitCode === null && child.signalCode === null) {
      const stopped = once(child, 'exit'); child.kill(); await stopped;
    }
    for (const socket of connections) socket.destroy();
    await new Promise(resolve => vnc.close(resolve));
  });
  let response;
  for (let i = 0; i < 60; i++) {
    try { response = await fetch('http://127.0.0.1:6080/vnc.html'); break; }
    catch { await delay(50); }
  }
  assert.ok(response, errors || 'Preview did not start');
  assert.equal(response.status, 200);
  assert.match(await response.text(), /noVNC/);
  const denied = await fetch('http://127.0.0.1:6080/..%2F..%2Fetc%2Fpasswd');
  assert.equal(denied.status, 404);
  ws = new WebSocket('ws://127.0.0.1:6080/websockify', ['binary']);
  const greeting = once(ws, 'message');
  await once(ws, 'open');
  const [bytes, binary] = await greeting;
  assert.equal(binary, true); assert.equal(bytes.toString(), 'RFB 003.008\n');
  const echoed = once(ws, 'message');
  ws.send(Buffer.from([0, 1, 2, 255]));
  assert.deepEqual((await echoed)[0], Buffer.from([0, 1, 2, 255]));
  const closed = once(ws, 'close'); ws.close(); await closed;
  assert.equal(errors, '');
});
