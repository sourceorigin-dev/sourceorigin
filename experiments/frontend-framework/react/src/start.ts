import { contentSecurityPolicy } from '@experiment/shared/csp';
import { createMiddleware, createStart } from '@tanstack/react-start';

// Every response gets the Content Security Policy, with a nonce of its own.
const contentSecurity = createMiddleware().server(async ({ next, request }) => {
  const nonce = crypto.randomUUID().replaceAll('-', '');
  const result = await next({ context: { nonce } });
  result.response.headers.set(
    'Content-Security-Policy',
    contentSecurityPolicy(nonce, new URL(request.url).pathname)
  );
  return result;
});

export const startInstance = createStart(() => ({
  requestMiddleware: [contentSecurity],
}));
