// Serves the production build: `node server.js`. adapter-node's own server
// (`node build`) would do, but it serves files without the Content Security
// Policy, and a worker runs under the policy its own script came with. So this
// puts the policy on every answer, then hands over to adapter-node's handler;
// SvelteKit replaces it on pages with its own, which carries the nonce.
import { createServer } from 'node:http';

import { contentSecurityPolicy } from '../shared/src/csp.ts';
import { handler } from './build/handler.js';

const port = Number(process.env.PORT ?? 4302);

createServer((request, response) => {
  const { pathname } = new URL(request.url, `http://${request.headers.host}`);
  response.setHeader('content-security-policy', contentSecurityPolicy('', pathname));
  handler(request, response, () => {
    response.statusCode = 404;
    response.end('Not found');
  });
}).listen(port, '127.0.0.1', () => console.log(`SvelteKit on http://127.0.0.1:${port}`));
