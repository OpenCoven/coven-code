// Child-process preload for controlled installer inputs on the Windows runner.
const fs = require('node:fs');
const http = require('node:http');
const https = require('node:https');
const { EventEmitter } = require('node:events');

https.get = (url, callback) => {
  if (process.env.INSTALL_CASE === 'offline') {
    throw new Error('Existing-binary check attempted a download');
  }
  if (url !== 'https://github.com/OpenCoven/coven-code/releases/download/v0.9.3/coven-code-windows-x86_64.zip') {
    throw new Error('Unexpected archive URL: ' + url);
  }
  if (process.env.INSTALL_CASE === 'timeout') {
    // Keep a real socket open without headers until the installer's own timer fires.
    const server = http.createServer(() => {});
    const request = new EventEmitter();
    server.listen(0, '127.0.0.1', () => {
      const pending = http.get({ host: '127.0.0.1', port: server.address().port }, callback);
      pending.on('error', err => request.emit('error', err));
      request.destroy = err => { pending.destroy(err); server.close(); };
    });
    return request;
  }
  const response = fs.createReadStream(process.env.INSTALL_ARCHIVE);
  response.statusCode = 200;
  response.headers = {};
  process.nextTick(() => callback(response));
  return new EventEmitter();
};
