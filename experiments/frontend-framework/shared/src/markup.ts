// The markup Pierre rendered on the server, kept beside the page's data
// instead of in it. Both frameworks send a page's data to the browser a second
// time, as a script, so the page can wake up; markup put in the data would
// travel twice, and the browser has no use for the second copy: the parser has
// already turned it into shadow roots. So the loader files it here under the
// change it belongs to, and the components look it up while the server
// renders. In the browser nothing was filed, and they get nothing.

/** One file the server rendered: its path, and the markup of its shadow root. */
export interface ServerFile {
  path: string;
  html: string;
}

/** What the server rendered with Pierre's entry points for one change. */
export interface ServerMarkup {
  files: ServerFile[];
  /** The file tree's shadow root. */
  tree: string;
}

const rendered = new WeakMap<object, ServerMarkup>();

/** Files a change's markup, for as long as the change's data lives. */
export function keepServerMarkup(change: object, markup: ServerMarkup): void {
  rendered.set(change, markup);
}

/** The markup filed for a change: on the server only, and only for this request's data. */
export function serverMarkup(change: object): ServerMarkup | undefined {
  return rendered.get(change);
}
