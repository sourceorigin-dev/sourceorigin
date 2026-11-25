<!--
  Pierre's CodeView for Svelte: our own wrapper over the plain JavaScript class,
  the counterpart of the `CodeView` Pierre ships for React. It covers what this
  screen uses: items owned by the viewer, annotations, and a file's header.
  Controlled items, selection, editing and the gutter slot are not wrapped.
-->
<script lang="ts" generics="A">
  import {
    CodeView,
    type CodeViewItem,
    type CodeViewOptions,
    type CodeViewSlotSnapshot,
    type DiffLineAnnotation,
    HEADER_METADATA_SLOT_ID,
    type LineAnnotation,
  } from '@pierre/diffs';
  import type { WorkerPoolManager } from '@pierre/diffs/worker';
  import { flushSync, type Snippet } from 'svelte';
  import type { Attachment } from 'svelte/attachments';

  interface Props {
    class?: string;
    /** The items to start with. Later ones are added and changed through `viewer`. */
    initialItems: readonly CodeViewItem<A>[];
    /** Read once, when the viewer is made. */
    options: CodeViewOptions<A, undefined>;
    pool?: WorkerPoolManager;
    /** Pierre's viewer, once it exists. */
    viewer?: CodeView<A>;
    /** The element that scrolls. */
    scroller?: HTMLElement;
    /** What the row under an annotated line shows. */
    annotation?: Snippet<[DiffLineAnnotation<A> | LineAnnotation<A>, CodeViewItem<A>]>;
    /** What sits at the end of a file's header. */
    headerMetadata?: Snippet<[CodeViewItem<A>]>;
  }

  let {
    class: className,
    initialItems,
    options,
    pool,
    viewer = $bindable(),
    scroller = $bindable(),
    annotation,
    headerMetadata,
  }: Props = $props();

  // The files CodeView has in the page right now, each with the element whose
  // slots take our content. CodeView publishes a new one as files scroll in and out.
  let snapshot = $state.raw<CodeViewSlotSnapshot<A, undefined>>();

  const mount: Attachment<HTMLElement> = (root) => {
    // Svelte is already flushing while this runs, and flushSync cannot nest.
    let mounting = true;
    const instance = new CodeView<A>(options, pool, true);
    instance.setup(root);
    instance.setItems(initialItems);
    instance.setSlotCoordinator({
      hasHeaderRenderers: headerMetadata != null,
      hasAnnotationRenderer: annotation != null,
      hasGutterRenderer: false,
      onSnapshotChange(next) {
        // CodeView measures a file right after rendering it, so its slots must
        // be filled before this returns.
        if (mounting) snapshot = next;
        else flushSync(() => (snapshot = next));
      },
    });
    instance.render(true);
    mounting = false;
    viewer = instance;
    scroller = root;
    return () => {
      instance.cleanUp();
      viewer = undefined;
      snapshot = undefined;
    };
  };

  /**
   * Moves a node Svelte rendered into a file's element and names its slot,
   * which places it inside Pierre's shadow root. Svelte still owns the node: it
   * updates it in place and removes it wherever it is. The slot is set here
   * because Svelte's compiler refuses a `slot` attribute on an element that is
   * not written inside a custom element.
   */
  const into =
    (file: HTMLElement, slot: string): Attachment<HTMLElement> =>
    (node) => {
      node.slot = slot;
      file.append(node);
    };
</script>

<div class={className} {@attach mount}></div>

<div hidden>
  {#each snapshot?.items ?? [] as rendered (rendered.id)}
    {#if headerMetadata}
      <div {@attach into(rendered.element, HEADER_METADATA_SLOT_ID)}>
        {@render headerMetadata(rendered.item)}
      </div>
    {/if}
    {#if annotation}
      <!-- Not keyed: two annotations on one line share a slot name. -->
      {#each rendered.item.annotations ?? [] as line}
        <div {@attach into(rendered.element, rendered.instance.getAnnotationSlotName(line))}>
          {@render annotation(line, rendered.item)}
        </div>
      {/each}
    {/if}
  {/each}
</div>
