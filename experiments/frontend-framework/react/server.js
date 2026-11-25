// Serves the production build: `node server.js`. TanStack Start builds a fetch
// handler and leaves serving it on Node to Nitro, which its docs install from a
// nightly build; these lines do the same job without it.
import { createReadStream, existsSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize } from 'node:path';
import { Readable } from 'node:stream';

import { contentSecurityPolicy } from '../shared/src/csp.ts';
import app from './dist/server/server.js';

const CLIENT = join(import.meta.dirname, 'dist/client');
const TYPES = {
  '.js': 'text/javascript',
  '.css': 'text/css',
  '.woff2': 'font/woff2',
  '.wasm': 'application/wasm',
  '.txt': 'text/plain',
};
const port = Number(process.env.PORT ?? 4301);

createServer(async (request, response) => {
  const url = new URL(request.url, `http://${request.headers.host}`);
  const file = join(CLIENT, normalize(url.pathname));
  if (file.startsWith(CLIENT) && existsSync(file) && statSync(file).isFile()) {
    // A worker runs under the policy its own script came with, so files get one too.
    response.writeHead(200, {
      'content-type': TYPES[extname(file)] ?? 'application/octet-stream',
      'cache-control': 'public, max-age=31536000, immutable',
      'content-security-policy': contentSecurityPolicy('', url.pathname),
    });
    createReadStream(file).pipe(response);
    return;
  }
  const body = ['GET', 'HEAD'].includes(request.method) ? undefined : Readable.toWeb(request);
  const answer = await app.fetch(
    new Request(url, { method: request.method, headers: request.headers, body, duplex: 'half' })
  );
  response.writeHead(answer.status, Object.fromEntries(answer.headers));
  if (answer.body == null) response.end();
  else Readable.fromWeb(answer.body).pipe(response);
}).listen(port, '127.0.0.1', () => console.log(`React with TanStack Start on http://127.0.0.1:${port}`));
