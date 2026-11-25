<script lang="ts">
  import Review from '#lib/review/Review.svelte';

  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
</script>

<svelte:head><title>{data.change.title} · SourceOrigin</title></svelte:head>

<Review {data} />
