<!--
  Pierre's file tree for Svelte, over its plain JavaScript class. After a full
  page load it takes over the markup the server rendered; after a navigation in
  the browser there is none, and it renders from scratch.
-->
<script lang="ts">
  import { FileTree, type FileTreeOptions } from '@pierre/trees';
  import type { Attachment } from 'svelte/attachments';

  import { browser } from '$app/env';

  interface Props {
    /** Read once, when the tree is made. */
    options: FileTreeOptions;
    /** What `preloadFileTree` rendered on the server. */
    preloaded?: { id: string; shadowHtml: string };
  }

  let { options, preloaded }: Props = $props();

  const mount: Attachment<HTMLElement> = (host) => {
    const tree = new FileTree(options);
    if (host.shadowRoot?.querySelector('[data-file-tree-id]') != null) {
      tree.hydrate({ fileTreeContainer: host });
    } else {
      tree.render({ fileTreeContainer: host });
    }
    return () => tree.cleanUp();
  };
</script>

<file-tree-container id={preloaded?.id} {@attach mount}>
  <!-- Only the server writes the template: the browser turns it into the shadow
       root while parsing, and Svelte leaves markup from {@html} as it finds it. -->
  <!-- svelte-ignore hydration_html_changed -->
  {@html browser || preloaded == null
    ? ''
    : `<template shadowrootmode="open">${preloaded.shadowHtml}</template>`}
</file-tree-container>
