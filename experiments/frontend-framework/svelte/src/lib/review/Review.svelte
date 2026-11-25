<!--
  The change review screen. CodeView owns its items, as Pierre recommends for a
  long list: the threads, the draft and the revisions are plain values, and each
  change to them updates the one item it touches. Svelte state holds only what
  Svelte draws itself: the reviewed marks, the notice and the handover from the
  server's files.
-->
<script lang="ts">
  import type { CodeView as Viewer, CodeViewOptions } from '@pierre/diffs';
  import { SvelteSet } from 'svelte/reactivity';

  import { browser } from '$app/env';
  import {
    addComment,
    type ChangeEvent,
    type Comment,
    createThread,
    type FileSummary,
    getFilePatch,
    getPatch,
    setReviewed as saveReviewed,
    type Side,
    simulate,
    streamEvents,
    type Thread,
  } from '@experiment/shared/api';
  import {
    annotationsFor,
    CODE_VIEW_OPTIONS,
    firstDiffLine,
    itemFromPatch,
    ME,
    type ReviewAnnotation,
    type ReviewItem,
    SERVER_RENDERED_FILES,
    splitPatch,
    streamPatchFiles,
    threadsByPath,
    treeOptions,
    workerPool,
  } from '@experiment/shared/review';
  import { serverMarkup } from '@experiment/shared/markup';
  import type { FirstScreen } from '@experiment/shared/server';

  import CodeView from '../pierre/CodeView.svelte';
  import FileTree from '../pierre/FileTree.svelte';
  import ServerFiles from '../pierre/ServerFiles.svelte';
  import CommentForm from './CommentForm.svelte';
  import ThreadView from './ThreadView.svelte';

  /** The comment being written: which line of which file it is on. */
  interface DraftLine {
    path: string;
    side: Side;
    line: number;
  }

  /** An update that arrived from someone else, and where to look for it. */
  interface Notice {
    text: string;
    path: string;
    line?: number;
  }

  let { data }: { data: FirstScreen } = $props();
  // The change never changes under a mounted page, so it is read once.
  // svelte-ignore state_referenced_locally
  const { change } = data;
  const path = { change: change.id };
  const threads = threadsByPath(data.threads);
  let draft: DraftLine | undefined;

  let viewer = $state.raw<Viewer<ReviewAnnotation>>();
  let scroller = $state.raw<HTMLElement>();
  const reviewed = new SvelteSet(change.files.filter((file) => file.reviewed).map((file) => file.path));
  let notice = $state<Notice>();
  // Whether CodeView has rendered and the server's files can go.
  let live = $state(false);

  /** A file's annotations as they stand now. */
  const annotations = (id: string) =>
    annotationsFor(threads.get(id) ?? [], draft?.path === id ? draft : undefined);

  /** Draws a file again after its threads, draft or anything in `patch` changed. */
  function refresh(id: string, patch?: Partial<ReviewItem>) {
    const item = viewer?.getItem(id);
    if (item?.type !== 'diff') return;
    viewer?.updateItem({
      ...item,
      ...patch,
      annotations: annotations(id),
      version: (item.version ?? 0) + 1,
    });
  }

  /** A file of the patch as an item, with its threads and its reviewed state. */
  function toItem(text: string, revision = 0): ReviewItem | undefined {
    const item = itemFromPatch(text, revision);
    if (item == null) return undefined;
    return { ...item, annotations: annotations(item.id), collapsed: reviewed.has(item.id) };
  }

  const initialItems = splitPatch(data.firstPatch).flatMap((text) => toItem(text) ?? []);
  // What Pierre rendered on the server. In the browser there is none, and the
  // markup already in the page stays as the parser left it.
  const markup = serverMarkup(change);
  const serverFiles = initialItems.map((item) => ({
    path: item.id,
    html: markup?.files.find((file) => file.path === item.id)?.html ?? '',
  }));

  /** Adds a thread unless it is already there, as its own echo over the stream is. */
  function addThread(thread: Thread): boolean {
    const list = threads.get(thread.path) ?? [];
    if (list.some((known) => known.id === thread.id)) return false;
    threads.set(thread.path, [...list, thread]);
    refresh(thread.path);
    return true;
  }

  function addReply(thread: number, comment: Comment) {
    for (const [file, list] of threads) {
      const index = list.findIndex((known) => known.id === thread);
      if (index === -1) continue;
      const known = list[index];
      if (known.comments.some((existing) => existing.id === comment.id)) return;
      threads.set(file, list.with(index, { ...known, comments: [...known.comments, comment] }));
      refresh(file);
      return;
    }
  }

  /** The author pushed a new version of a file: fetch its diff and show it. */
  async function revise(file: FileSummary) {
    const patch = await getFilePatch({ path, query: { path: file.path }, parseAs: 'text' });
    const item = patch.data == null ? undefined : toItem(patch.data as string, file.revision);
    if (item == null) return;
    reviewed.delete(file.path);
    refresh(file.path, { fileDiff: item.fileDiff, collapsed: false });
    notice = { text: `${change.author} pushed a new version of ${file.path}`, path: file.path };
  }

  function apply(event: ChangeEvent) {
    switch (event.type) {
      case 'thread_created': {
        const { thread } = event;
        const author = thread.comments[0]?.author;
        if (addThread(thread) && author !== ME) {
          notice = { text: `${author} commented on ${thread.path}`, path: thread.path, line: thread.line };
        }
        break;
      }
      case 'comment_added':
        addReply(event.thread_id, event.comment);
        break;
      case 'file_revised':
        void revise(event.file);
        break;
    }
  }

  // Once the viewer exists: the rest of the patch, the live events, and the
  // moment the server's files can go.
  $effect(() => {
    const target = viewer;
    const root = scroller;
    if (target == null || root == null) return;
    const abort = new AbortController();
    const { signal } = abort;
    void firstDiffLine(root, signal).then(() => (live = true));
    void (async () => {
      const rest = await getPatch({
        path,
        query: { offset: SERVER_RENDERED_FILES },
        parseAs: 'stream',
        signal,
      });
      if (rest.data == null) return;
      await streamPatchFiles(rest.data as unknown as ReadableStream<Uint8Array>, (texts) =>
        target.addItems(texts.flatMap((text) => toItem(text) ?? []))
      );
      performance.mark('review:all-files');
    })().catch(() => {});
    void (async () => {
      const events = await streamEvents({ path, signal });
      for await (const event of events.stream) apply(event);
    })().catch(() => {});
    return () => abort.abort();
  });

  function startDraft(next: DraftLine) {
    const previous = draft;
    draft = next;
    if (previous != null && previous.path !== next.path) refresh(previous.path);
    refresh(next.path);
  }

  function cancelDraft() {
    const previous = draft;
    draft = undefined;
    if (previous != null) refresh(previous.path);
  }

  async function submitDraft(body: string) {
    const line = draft;
    if (line == null) return;
    const created = await createThread({ path, body: { ...line, body } });
    if (created.data == null) throw new Error(created.error?.message ?? 'The comment was not saved.');
    draft = undefined;
    if (!addThread(created.data)) refresh(line.path);
  }

  async function reply(thread: number, body: string) {
    const added = await addComment({ path: { ...path, thread }, body: { body } });
    if (added.data == null) throw new Error(added.error?.message ?? 'The reply was not saved.');
    addReply(thread, added.data);
  }

  function markReviewed(id: string, value: boolean) {
    if (value) reviewed.add(id);
    else reviewed.delete(id);
    // A reviewed file folds away, as it would on any review page.
    refresh(id, { collapsed: value });
    void saveReviewed({ path, body: { path: id, reviewed: value } });
  }

  function show(target: Notice) {
    viewer?.scrollTo(
      target.line == null
        ? { type: 'item', id: target.path, align: 'start' }
        : { type: 'line', id: target.path, lineNumber: target.line, side: 'additions', align: 'center' }
    );
    notice = undefined;
  }

  const options: CodeViewOptions<ReviewAnnotation, undefined> = {
    ...CODE_VIEW_OPTIONS,
    onGutterUtilityClick(range, context) {
      startDraft({
        path: context.item.id,
        side: range.endSide ?? range.side ?? 'additions',
        line: range.end,
      });
    },
  };
</script>

<div class="review">
  <header class="review-bar">
    <nav class="trail" aria-label="Breadcrumb">
      <a href="/">SourceOrigin</a>
      <span aria-hidden="true">/</span>
      <span>sourceorigin</span>
      <span aria-hidden="true">/</span>
      <span>Change {change.id}</span>
      <span class="grow"></span>
      <a href="/settings">Account settings</a>
    </nav>
    <div>
      <h1>{change.title}</h1>
      <div class="summary">
        <span>
          {change.author} wants to land <span class="mono">{change.base}</span> to
          <span class="mono">{change.head}</span>
        </span>
        <span>{change.files.length.toLocaleString('en')} files</span>
        <span class="added">+{change.additions.toLocaleString('en')}</span>
        <span class="removed">−{change.deletions.toLocaleString('en')}</span>
        <button
          type="button"
          class="text-button small"
          onclick={() => void simulate({ path, body: { kind: 'thread' } })}
        >
          Simulate a comment
        </button>
        <button
          type="button"
          class="text-button small"
          onclick={() => void simulate({ path, body: { kind: 'revision' } })}
        >
          Simulate a new version
        </button>
      </div>
    </div>
    <div class="progress">
      <span><strong>{reviewed.size}</strong> of {change.files.length} files reviewed</span>
      <progress aria-label="Files reviewed" value={reviewed.size} max={change.files.length}></progress>
    </div>
  </header>

  <div class="review-body">
    <aside class="review-tree" aria-label="Files">
      <FileTree
        preloaded={{ id: data.treeId, shadowHtml: markup?.tree ?? '' }}
        options={{
          ...treeOptions(change.files),
          onSelectionChange(paths) {
            if (paths.length === 1 && viewer?.getItem(paths[0]) != null) {
              viewer.scrollTo({ type: 'item', id: paths[0], align: 'start' });
            }
          },
        }}
      />
    </aside>
    <main class="review-diffs">
      {#if browser}
        <CodeView
          bind:viewer
          bind:scroller
          class="review-diff"
          {initialItems}
          {options}
          pool={workerPool()}
        >
          {#snippet annotation(line)}
            {#if line.metadata.kind === 'draft'}
              <section class="thread" aria-label="New comment">
                <CommentForm
                  label="Comment on this line"
                  submit="Comment"
                  autofocus
                  oncancel={cancelDraft}
                  onsubmit={submitDraft}
                />
              </section>
            {:else}
              <ThreadView thread={line.metadata.thread} onreply={reply} />
            {/if}
          {/snippet}
          {#snippet headerMetadata(item)}
            <label class="file-mark">
              <input
                type="checkbox"
                checked={reviewed.has(item.id)}
                onchange={(event) => markReviewed(item.id, event.currentTarget.checked)}
              />
              Reviewed
            </label>
          {/snippet}
        </CodeView>
      {/if}
      {#if !live}
        <ServerFiles files={serverFiles} {threads}>
          {#snippet thread(value)}
            <ThreadView thread={value} />
          {/snippet}
        </ServerFiles>
      {/if}
    </main>
  </div>

  {#if notice}
    {@const current = notice}
    <div class="notice info live" role="status">
      <div class="text">{current.text}</div>
      <button type="button" class="text-button small" onclick={() => show(current)}>Show</button>
      <button
        type="button"
        class="text-button small"
        aria-label="Dismiss"
        onclick={() => (notice = undefined)}
      >
        ×
      </button>
    </div>
  {/if}
</div>
