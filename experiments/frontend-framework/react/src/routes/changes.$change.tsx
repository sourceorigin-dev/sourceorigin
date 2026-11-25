import { loadFirstScreen } from '@experiment/shared/server';
import { createFileRoute } from '@tanstack/react-router';
import { createServerFn } from '@tanstack/react-start';

import { Review } from '../review/Review';

// Runs on the server only: for the first request, and as a call from the
// browser when someone navigates here from another page.
const firstScreen = createServerFn()
  .validator((change: string) => change)
  .handler(({ data }) => loadFirstScreen(data));

export const Route = createFileRoute('/changes/$change')({
  loader: ({ params }) => firstScreen({ data: params.change }),
  head: ({ loaderData }) => ({
    meta: [{ title: `${loaderData?.change.title ?? 'Change'} · SourceOrigin` }],
  }),
  component: ReviewPage,
});

function ReviewPage() {
  // Not passed straight into the prop: the hook's type follows the place it is
  // used, so `<Review data={Route.useLoaderData()} />` would accept any loader.
  const data = Route.useLoaderData();
  return <Review data={data} />;
}
