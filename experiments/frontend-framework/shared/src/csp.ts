// The Content Security Policy both candidates send. It starts from what Origin
// Auth sends today (`default-src 'none'`, no inline anything) and adds only
// what the experiment showed is needed; each addition says why.

/** Where the stub API listens. A real deployment would configure this. */
export const API_URL = 'http://127.0.0.1:4100';

export const CSP_DIRECTIVES: Record<string, string[]> = {
  'default-src': ["'none'"],
  // Scripts from this origin, plus the nonce each framework puts on the inline
  // script that starts hydration.
  'script-src': ["'self'"],
  'style-src': ["'self'"],
  'font-src': ["'self'"],
  'img-src': ["'self'"],
  // The API, called from the browser.
  'connect-src': ["'self'", API_URL],
  // Pierre's highlighting workers.
  'worker-src': ["'self'"],
  'frame-ancestors': ["'none'"],
  'base-uri': ["'none'"],
  'form-action': ["'self'"],
};

/**
 * What a page with Pierre's components needs on top: they write `style`
 * attributes and `<style>` elements, in the markup the server renders and from
 * their scripts, and take no nonce. A nonce in `style-src` would switch
 * 'unsafe-inline' off, so these pages get none there.
 */
const PIERRE_STYLE_SRC = ["'self'", "'unsafe-inline'"];

/**
 * And for scripts: `@pierre/highlights` is WebAssembly, which a policy without
 * this refuses. Pierre then falls back to Shiki's JavaScript engine without a
 * word, so the page works and is slower. It lets WebAssembly compile, not
 * `eval` run.
 */
export const PIERRE_SCRIPT_SRC = "'wasm-unsafe-eval'";

/** Whether a path shows Pierre's components, and so needs the looser styles. */
export function usesPierre(pathname: string): boolean {
  return pathname.startsWith('/changes/');
}

/**
 * The header's value for a page, with `nonce` allowed for its inline scripts.
 * A file gets the same policy with no nonce: pass an empty one.
 */
export function contentSecurityPolicy(nonce: string, pathname: string): string {
  return Object.entries(CSP_DIRECTIVES)
    .map(([directive, sources]) => {
      if (directive === 'style-src' && usesPierre(pathname)) {
        return [directive, ...PIERRE_STYLE_SRC].join(' ');
      }
      const nonced = directive === 'script-src' || directive === 'style-src';
      // A file's policy matters to one reader, the worker started from it, and
      // Pierre's workers are where most highlighting runs.
      const wasm = directive === 'script-src' && (nonce === '' || usesPierre(pathname));
      return [
        directive,
        ...sources,
        ...(wasm ? [PIERRE_SCRIPT_SRC] : []),
        ...(nonced && nonce !== '' ? [`'nonce-${nonce}'`] : []),
      ].join(' ');
    })
    .join('; ');
}
