/**
 * The transport policy: a deadline and no redirects, whatever the injected `fetch`
 * does, and network errors that name the URL.
 */
import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { AuthError, Issuer } from '../src/index.ts';
import { Http } from '../src/http.ts';
import { DISCOVERY_URL, ISSUER } from './vectors.ts';

const TOKEN_URL = 'https://auth.example.com/oauth/token';

/** Whether `error` is a `network` error. */
const network = (error: unknown) => error instanceof AuthError && error.code === 'network';

test('a server that never answers times out, though the fetch ignores its signal', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    // A fetch that never settles, whatever its signal says.
    const http = new Http(() => new Promise<Response>(() => undefined));
    const sent = http.get(TOKEN_URL);
    mock.timers.tick(10_000);
    await assert.rejects(sent, network);
  } finally {
    mock.timers.reset();
  }
});

/** A body that never sends a byte, and records whether it was cancelled. */
class StalledBody {
  cancelled = false;
  readonly stream = new ReadableStream<Uint8Array>({
    cancel: () => {
      this.cancelled = true;
    }
  });
}

test('a body that stalls times out, and is cancelled', async () => {
  mock.timers.enable({ apis: ['setTimeout'] });
  try {
    const body = new StalledBody();
    const http = new Http(() => Promise.resolve(new Response(body.stream)));
    const sent = http.get(TOKEN_URL);
    // Let the response arrive and its body start being read.
    await new Promise((resolve) => setImmediate(resolve));
    mock.timers.tick(10_000);
    await assert.rejects(sent, network);
    assert.equal(body.cancelled, true);
  } finally {
    mock.timers.reset();
  }
});

test('a redirect is refused and its body cancelled, and it is never asked to be followed', async () => {
  let redirect: RequestRedirect | undefined;
  const body = new StalledBody();
  const http = new Http((_, init) => {
    redirect = init.redirect;
    return Promise.resolve(
      new Response(body.stream, {
        status: 307,
        headers: { location: 'https://elsewhere.example' }
      })
    );
  });
  await assert.rejects(http.post(TOKEN_URL, { refresh_token: 'prt_v1_secret' }), network);
  assert.equal(redirect, 'manual');
  assert.equal(body.cancelled, true);
});

test('a response a fetch reached by following a redirect is refused', async () => {
  const http = new Http(() => {
    const followed = Response.json({ access_token: 'stolen' });
    Object.defineProperty(followed, 'redirected', { value: true });
    return Promise.resolve(followed);
  });
  await assert.rejects(http.get(TOKEN_URL), network);
});

test('only https URLs are requested', async () => {
  let sent = false;
  const http = new Http(() => {
    sent = true;
    return Promise.resolve(new Response('{}'));
  });
  await assert.rejects(http.get('http://auth.example.com/jwks'), network);
  assert.equal(sent, false);
});

test('a document endpoint answering other than 200 fails, naming the URL', async () => {
  const fetch = () => Promise.resolve(new Response(null, { status: 500 }));
  await assert.rejects(Issuer.discover(ISSUER, { fetch }), { code: 'network', url: DISCOVERY_URL });
});

test('a body over 256 KiB fails, naming the URL', async () => {
  const http = new Http(() => Promise.resolve(new Response(new Uint8Array(256 * 1024 + 1))));
  await assert.rejects(http.get(TOKEN_URL), { code: 'network', url: TOKEN_URL });
});
