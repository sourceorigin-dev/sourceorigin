import { createRouter } from '@tanstack/react-router';
import { createIsomorphicFn, getGlobalStartContext } from '@tanstack/react-start';

import { routeTree } from './routeTree.gen';

// The nonce the CSP middleware in start.ts made for this request. The router
// puts it on every inline script and style it writes. In the browser the router
// reads it back from a <meta> tag by itself.
const requestNonce = createIsomorphicFn()
  .server(() => getGlobalStartContext()?.nonce)
  .client(() => undefined);

export function getRouter() {
  return createRouter({
    routeTree,
    scrollRestoration: true,
    defaultPreload: 'intent',
    ssr: { nonce: requestNonce() },
  });
}
