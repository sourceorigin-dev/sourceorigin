/** The time source for every expiry and cache check: milliseconds since the Unix epoch, as `Date.now` returns. Inject one to pin time in tests. */
export type Clock = () => number;
