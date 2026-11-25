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
      return [directive, ...sources, ...(nonced && nonce !== '' ? [`'nonce-${nonce}'`] : [])].join(' ');
    })
    .join('; ');
}
