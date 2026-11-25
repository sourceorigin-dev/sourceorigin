import { loadFirstScreen } from '@experiment/shared/server';

import type { PageServerLoad } from './$types';

// Runs on the server only: for the first request, and as a request from the
// browser when someone navigates here from another page.
export const load: PageServerLoad = ({ params }) => loadFirstScreen(params.change);
