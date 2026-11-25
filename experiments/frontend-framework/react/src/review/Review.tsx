import type { CodeViewOptions } from '@pierre/diffs';
import {
  CodeView,
  type CodeViewAnnotationArgs,
  type CodeViewHandle,
  WorkerPoolContext,
} from '@pierre/diffs/react';
import { FileTree, useFileTree } from '@pierre/trees/react';
import { Link } from '@tanstack/react-router';
import { useEffect, useMemo, useRef, useState } from 'react';

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

import { ServerFiles } from '../pierre/ServerFiles';
import { Draft, ThreadView } from './Thread';

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

/**
 * The change review screen. CodeView owns its items, as Pierre recommends for
 * a long list: the threads, the draft and the revisions live in refs, and each
 * change to them updates the one item it touches. React state holds only what
 * React draws itself: the reviewed marks, the notice and the handover from the
 * server's files.
 */
export function Review({ data }: { data: FirstScreen }) {
  const { change } = data;
  const path = { change: change.id };
  const viewer = useRef<CodeViewHandle<ReviewAnnotation, undefined>>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const threads = useRef(threadsByPath(data.threads));
  const draft = useRef<DraftLine | null>(null);
  const [reviewed, setReviewed] = useState(
    () => new Set(change.files.filter((file) => file.reviewed).map((file) => file.path))
  );
  const [notice, setNotice] = useState<Notice | null>(null);
  // Whether CodeView has rendered and the server's files can go.
  const [live, setLive] = useState(false);
  const [pool] = useState(() => (typeof window === 'undefined' ? undefined : workerPool()));
  // What Pierre rendered on the server. In the browser there is none, and the
  // markup already in the page stays as the parser left it.
  const markup = serverMarkup(change);

  /** A file's annotations as they stand now. */
  const annotations = (id: string) =>
    annotationsFor(
      threads.current.get(id) ?? [],
      draft.current?.path === id ? draft.current : undefined
    );

  /** Draws a file again after its threads, draft or anything in `patch` changed. */
  function refresh(id: string, patch?: Partial<ReviewItem>) {
    const item = viewer.current?.getItem(id);
    if (item?.type !== 'diff') return;
    viewer.current?.updateItem({
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

  const [initialItems] = useState(() =>
    splitPatch(data.firstPatch).flatMap((text) => toItem(text) ?? [])
  );

  /** Adds a thread unless it is already there, as its own echo over the stream is. */
  function addThread(thread: Thread): boolean {
    const list = threads.current.get(thread.path) ?? [];
    if (list.some((known) => known.id === thread.id)) return false;
    threads.current.set(thread.path, [...list, thread]);
    refresh(thread.path);
    return true;
  }

  function addReply(thread: number, comment: Comment) {
    for (const [file, list] of threads.current) {
      const index = list.findIndex((known) => known.id === thread);
      if (index === -1) continue;
      const known = list[index];
      if (known.comments.some((existing) => existing.id === comment.id)) return;
      threads.current.set(
        file,
        list.with(index, { ...known, comments: [...known.comments, comment] })
      );
      refresh(file);
      return;
    }
  }

  /** The author pushed a new version of a file: fetch its diff and show it. */
  async function revise(file: FileSummary) {
    const patch = await getFilePatch({ path, query: { path: file.path }, parseAs: 'text' });
    const item = patch.data == null ? undefined : toItem(patch.data as string, file.revision);
    if (item == null) return;
    setReviewed((marks) => {
      const next = new Set(marks);
      next.delete(file.path);
      return next;
    });
    refresh(file.path, { fileDiff: item.fileDiff, collapsed: false });
    setNotice({ text: `${change.author} pushed a new version of ${file.path}`, path: file.path });
  }

  function apply(event: ChangeEvent) {
    switch (event.type) {
      case 'thread_created': {
        const { thread } = event;
        const author = thread.comments[0]?.author;
        if (addThread(thread) && author !== ME) {
          setNotice({
            text: `${author} commented on ${thread.path}`,
            path: thread.path,
            line: thread.line,
          });
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
  // The stream outlives every render, so it calls whichever `apply` is current.
  const applyLatest = useRef(apply);
  applyLatest.current = apply;
  const toItemLatest = useRef(toItem);
  toItemLatest.current = toItem;

  useEffect(() => {
    const abort = new AbortController();
    const { signal } = abort;
    if (scroller.current != null) {
      void firstDiffLine(scroller.current, signal).then(() => setLive(true));
    }
    void (async () => {
      const rest = await getPatch({
        path,
        query: { offset: SERVER_RENDERED_FILES },
        parseAs: 'stream',
        signal,
      });
      if (rest.data == null) return;
      await streamPatchFiles(rest.data as unknown as ReadableStream<Uint8Array>, (texts) =>
        viewer.current?.addItems(texts.flatMap((text) => toItemLatest.current(text) ?? []))
      );
      performance.mark('review:all-files');
    })().catch(() => {});
    void (async () => {
      const events = await streamEvents({ path, signal });
      for await (const event of events.stream) applyLatest.current(event);
    })().catch(() => {});
    return () => abort.abort();
    // The change never changes under a mounted page.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function startDraft(next: DraftLine) {
    const previous = draft.current;
    draft.current = next;
    if (previous != null && previous.path !== next.path) refresh(previous.path);
    refresh(next.path);
  }

  function cancelDraft() {
    const previous = draft.current;
    draft.current = null;
    if (previous != null) refresh(previous.path);
  }

  async function submitDraft(body: string) {
    const line = draft.current;
    if (line == null) return;
    const created = await createThread({ path, body: { ...line, body } });
    if (created.data == null) throw new Error(created.error?.message ?? 'The comment was not saved.');
    draft.current = null;
    if (!addThread(created.data)) refresh(line.path);
  }

  async function reply(thread: number, body: string) {
    const added = await addComment({ path: { ...path, thread }, body: { body } });
    if (added.data == null) throw new Error(added.error?.message ?? 'The reply was not saved.');
    addReply(thread, added.data);
  }

  function markReviewed(id: string, value: boolean) {
    setReviewed((marks) => {
      const next = new Set(marks);
      if (value) next.add(id);
      else next.delete(id);
      return next;
    });
    // A reviewed file folds away, as it would on any review page.
    refresh(id, { collapsed: value });
    void saveReviewed({ path, body: { path: id, reviewed: value } });
  }

  function show(target: Notice) {
    viewer.current?.scrollTo(
      target.line == null
        ? { type: 'item', id: target.path, align: 'start' }
        : { type: 'line', id: target.path, lineNumber: target.line, side: 'additions', align: 'center' }
    );
    setNotice(null);
  }

  const startDraftLatest = useRef(startDraft);
  startDraftLatest.current = startDraft;
  // Pierre's wrapper compares options by identity, so these are made once and
  // reach the current handlers through refs.
  const options = useMemo<CodeViewOptions<ReviewAnnotation, undefined>>(
    () => ({
      ...CODE_VIEW_OPTIONS,
      onGutterUtilityClick(range, context) {
        startDraftLatest.current({
          path: context.item.id,
          side: range.endSide ?? range.side ?? 'additions',
          line: range.end,
        });
      },
    }),
    []
  );

  return (
    <div className="review">
      <header className="review-bar">
        <nav className="trail" aria-label="Breadcrumb">
          <Link to="/">SourceOrigin</Link>
          <span aria-hidden="true">/</span>
          <span>sourceorigin</span>
          <span aria-hidden="true">/</span>
          <span>Change {change.id}</span>
          <span className="grow" />
          <Link to="/settings">Account settings</Link>
        </nav>
        <div>
          <h1>{change.title}</h1>
          <div className="summary">
            <span>
              {change.author} wants to land <span className="mono">{change.base}</span> to{' '}
              <span className="mono">{change.head}</span>
            </span>
            <span>{change.files.length.toLocaleString('en')} files</span>
            <span className="added">+{change.additions.toLocaleString('en')}</span>
            <span className="removed">−{change.deletions.toLocaleString('en')}</span>
            <button
              type="button"
              className="text-button small"
              onClick={() => void simulate({ path, body: { kind: 'thread' } })}
            >
              Simulate a comment
            </button>
            <button
              type="button"
              className="text-button small"
              onClick={() => void simulate({ path, body: { kind: 'revision' } })}
            >
              Simulate a new version
            </button>
          </div>
        </div>
        <div className="progress">
          <span>
            <strong>{reviewed.size}</strong> of {change.files.length} files reviewed
          </span>
          <progress aria-label="Files reviewed" value={reviewed.size} max={change.files.length} />
        </div>
      </header>

      <div className="review-body">
        <aside className="review-tree" aria-label="Files">
          <Files
            files={change.files}
            tree={{ id: data.treeId, shadowHtml: markup?.tree ?? '' }}
            onSelect={(file) => {
              if (viewer.current?.getItem(file) != null) {
                viewer.current.scrollTo({ type: 'item', id: file, align: 'start' });
              }
            }}
          />
        </aside>
        <main className="review-diffs">
          <WorkerPoolContext value={pool}>
            <CodeView<ReviewAnnotation>
              ref={viewer}
              containerRef={scroller}
              className="review-diff"
              initialItems={initialItems}
              options={options}
              renderAnnotation={(...[annotation]: CodeViewAnnotationArgs<ReviewAnnotation>) =>
                annotation.metadata.kind === 'draft' ? (
                  <Draft onCancel={cancelDraft} onSubmit={submitDraft} />
                ) : (
                  <ThreadView thread={annotation.metadata.thread} onReply={reply} />
                )
              }
              renderHeaderMetadata={(item) => (
                <label className="file-mark">
                  <input
                    type="checkbox"
                    checked={reviewed.has(item.id)}
                    onChange={(event) => markReviewed(item.id, event.currentTarget.checked)}
                  />
                  Reviewed
                </label>
              )}
            />
          </WorkerPoolContext>
          {!live && (
            <ServerFiles
              files={initialItems.map((item) => ({
                path: item.id,
                html: markup?.files.find((file) => file.path === item.id)?.html ?? '',
              }))}
              threads={threads.current}
              renderThread={(thread) => <ThreadView thread={thread} />}
            />
          )}
        </main>
      </div>

      {notice != null && (
        <div className="notice info live" role="status">
          <div className="text">{notice.text}</div>
          <button type="button" className="text-button small" onClick={() => show(notice)}>
            Show
          </button>
          <button
            type="button"
            className="text-button small"
            aria-label="Dismiss"
            onClick={() => setNotice(null)}
          >
            ×
          </button>
        </div>
      )}
    </div>
  );
}

interface FilesProps {
  files: FileSummary[];
  tree: { id: string; shadowHtml: string };
  onSelect(path: string): void;
}

/** The file tree, taking over the markup the server rendered for it. */
function Files({ files, tree, onSelect }: FilesProps) {
  const select = useRef(onSelect);
  select.current = onSelect;
  const { model } = useFileTree({
    ...treeOptions(files),
    onSelectionChange(paths) {
      if (paths.length === 1) select.current(paths[0]);
    },
  });
  return <FileTree model={model} preloadedData={tree} />;
}
