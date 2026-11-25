<!--
  The files the server rendered, each in a declarative shadow root, with their
  threads in the slots Pierre's markup leaves for them. They are what a search
  engine reads and what shows before any script runs; the page removes them
  once CodeView has rendered.
-->
<script lang="ts">
  import { getLineAnnotationName } from '@pierre/diffs';
  import type { Snippet } from 'svelte';

  import { browser } from '$app/env';
  import type { Thread } from '@experiment/shared/api';
  import { annotationsFor, type ReviewAnnotation } from '@experiment/shared/review';
  import type { ServerFile } from '@experiment/shared/markup';

  interface Props {
    files: ServerFile[];
    threads: Map<string, Thread[]>;
    thread: Snippet<[Thread]>;
  }

  let { files, threads, thread }: Props = $props();
</script>

<div class="review-diff server-files">
  {#each files as file (file.path)}
    <diffs-container>
      <!-- svelte-ignore hydration_html_changed -->
      {@html browser ? '' : `<template shadowrootmode="open">${file.html}</template>`}
      {#each annotationsFor(threads.get(file.path) ?? []) as annotation}
        {#if annotation.metadata.kind === 'thread'}
          <div slot={getLineAnnotationName<ReviewAnnotation>(annotation)}>
            {@render thread(annotation.metadata.thread)}
          </div>
        {/if}
      {/each}
    </diffs-container>
  {/each}
</div>
