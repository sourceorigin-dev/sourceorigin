import type { Handle } from '@sveltejs/kit/hooks';

import { PIERRE_SCRIPT_SRC, usesPierre } from '@experiment/shared/csp';

// SvelteKit writes the Content Security Policy from vite.config.ts, the same
// for every page. Pages with Pierre's components need looser styles, and a
// nonce in style-src would switch 'unsafe-inline' off, so their style-src is
// rewritten here, and WebAssembly is allowed for Pierre's highlighter.
export const handle: Handle = async ({ event, resolve }) => {
  const response = await resolve(event, {
    // The generated client reads these two headers to parse a response, and
    // SvelteKit hides every header of a `load` request unless it is named here.
    filterSerializedResponseHeaders: (name) => name === 'content-type' || name === 'content-length',
  });
  const policy = response.headers.get('content-security-policy');
  if (policy != null && usesPierre(event.url.pathname)) {
    response.headers.set(
      'content-security-policy',
      policy
        .replace(/style-src [^;]*/, "style-src 'self' 'unsafe-inline'")
        .replace(/style-src-attr [^;]*; /, '')
        .replace('script-src ', `script-src ${PIERRE_SCRIPT_SRC} `)
    );
  }
  return response;
};
