import { getAccount } from '@experiment/shared/api';

import type { PageLoad } from './$types';

// Runs on the server for the first request and in the browser after that: the
// generated client is the same in both. It takes SvelteKit's `fetch`, so the
// answer the server got travels in the page and the browser does not ask again.
export const load: PageLoad = async ({ fetch }) => ({
  account: (await getAccount({ fetch, throwOnError: true })).data,
});
