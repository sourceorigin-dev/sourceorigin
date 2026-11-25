// What both candidates share that has nothing to do with a framework: turning
// the stub API's patch into CodeView items, threads into annotations, and the
// settings every Pierre component gets. State and rendering stay in each app.

import {
  canUseWASM,
  type CodeViewDiffItem,
  type CodeViewOptions,
  type DiffLineAnnotation,
  type HighlighterTypes,
  processFile,
} from '@pierre/diffs';
import type { FileTreeOptions, GitStatusEntry } from '@pierre/trees';
import {
  getOrCreateWorkerPoolSingleton,
  type WorkerPoolManager,
} from '@pierre/diffs/worker';

import type { FileSummary, Side, Thread } from './api';


/** The only change the stub API holds. */
export const CHANGE = '1';

/** How many files the server renders into the page before any script runs. */
export const SERVER_RENDERED_FILES = 3;

/** What a row under a diff line shows: a thread, or the comment being written. */
export type ReviewAnnotation =
  | { kind: 'thread'; thread: Thread }
  | { kind: 'draft'; side: Side; line: number };

export type ReviewItem = CodeViewDiffItem<ReviewAnnotation>;

/**
 * One file's part of a git patch as a CodeView item, without annotations.
 * `revision` is the file's revision from the API: it keys the highlight cache
 * and versions the item, so a new revision replaces the old diff.
 */
export function itemFromPatch(text: string, revision = 0): ReviewItem | undefined {
  const fileDiff = processFile(text, { isGitDiff: true });
  if (fileDiff == null) return undefined;
  fileDiff.cacheKey = `${fileDiff.name}@${revision}`;
  return { id: fileDiff.name, type: 'diff', fileDiff, version: revision };
}

/** Who the stub API takes the caller for: it has no sign-in. */
export const ME = 'mira';

/** A comment's time, written the same on the server and in any time zone. */
export function formatTime(seconds: number): string {
  return `${new Date(seconds * 1000).toISOString().slice(0, 16).replace('T', ' ')} UTC`;
}

/** The two letters an avatar shows. */
export function initials(name: string): string {
  return name.slice(0, 2).toUpperCase();
}

/**
 * Resolves once CodeView has put a diff line into `root`, and marks the moment
 * as `review:codeview` for the benchmark. Checked each frame: `onPostRender`
 * does not fire for the items of a CodeView.
 */
export function firstDiffLine(root: HTMLElement, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const look = (): void => {
      if (signal.aborted) return;
      const file = root.querySelector('diffs-container')?.shadowRoot;
      if (file?.querySelector('[data-column-number]') == null) {
        requestAnimationFrame(look);
        return;
      }
      performance.mark('review:codeview');
      resolve();
    };
    look();
  });
}

/** A file's threads, and the draft if it is on this file, as annotations. */
export function annotationsFor(
  threads: readonly Thread[],
  draft?: { side: Side; line: number }
): DiffLineAnnotation<ReviewAnnotation>[] {
  const annotations: DiffLineAnnotation<ReviewAnnotation>[] = threads.map((thread) => ({
    side: thread.side,
    lineNumber: thread.line,
    metadata: { kind: 'thread', thread },
  }));
  if (draft != null) {
    annotations.push({
      side: draft.side,
      lineNumber: draft.line,
      metadata: { kind: 'draft', ...draft },
    });
  }
  return annotations;
}

/** Threads grouped by the file they are on. */
export function threadsByPath(threads: readonly Thread[]): Map<string, Thread[]> {
  const byPath = new Map<string, Thread[]>();
  for (const thread of threads) {
    const list = byPath.get(thread.path);
    if (list == null) byPath.set(thread.path, [thread]);
    else list.push(thread);
  }
  return byPath;
}

const FILE_BOUNDARY = '\ndiff --git ';

/** A whole git patch as one text per file. */
export function splitPatch(patch: string): string[] {
  const files: string[] = [];
  let start = 0;
  for (let at = patch.indexOf(FILE_BOUNDARY); at !== -1; at = patch.indexOf(FILE_BOUNDARY, start)) {
    files.push(patch.slice(start, at + 1));
    start = at + 1;
  }
  if (start < patch.length) files.push(patch.slice(start));
  return files;
}

/**
 * Reads a git patch as it arrives and hands over the files each network chunk
 * completed, so the first files show before the last ones download.
 */
export async function streamPatchFiles(
  body: ReadableStream<Uint8Array>,
  onFiles: (texts: string[]) => void
): Promise<void> {
  const decoder = new TextDecoder();
  let pending = '';
  const drain = (last: boolean): void => {
    const files = splitPatch(pending);
    // The last file is still arriving until the stream ends.
    pending = last ? '' : (files.pop() ?? '');
    if (files.length > 0) onFiles(files);
  };
  // `for await` over a ReadableStream is not in Safari yet; a reader is.
  const reader = body.getReader();
  for (let chunk = await reader.read(); !chunk.done; chunk = await reader.read()) {
    pending += decoder.decode(chunk.value, { stream: true });
    drain(false);
  }
  pending += decoder.decode();
  drain(true);
}

/** The tree's git status for each file of the change. */
export function gitStatus(files: readonly FileSummary[]): GitStatusEntry[] {
  return files.map((file) => ({ path: file.path, status: file.status }));
}

/** The tree's options, the same on the server and in the browser. */
export function treeOptions(files: readonly FileSummary[]) {
  return {
    id: 'review-files',
    paths: files.map((file) => file.path),
    gitStatus: gitStatus(files),
    initialExpansion: 'open',
    flattenEmptyDirectories: true,
    stickyFolders: true,
    search: true,
    initialVisibleRowCount: 60,
  } as const satisfies FileTreeOptions;
}

/** The CodeView options that do not depend on the page's state. */
export const CODE_VIEW_OPTIONS = {
  theme: { light: 'pierre-light', dark: 'pierre-dark' },
  themeType: 'system',
  diffStyle: 'split',
  overflow: 'scroll',
  stickyHeaders: true,
  enableGutterUtility: true,
  lineHoverHighlight: 'number',
  layout: { paddingTop: 0, paddingBottom: 48, gap: 12 },
} as const satisfies CodeViewOptions<ReviewAnnotation, undefined>;

/**
 * Which highlighter to use, chosen as diffshub chooses: `@pierre/highlights`
 * where WebAssembly may run, which needs 'wasm-unsafe-eval' in the policy, and
 * Shiki's JavaScript engine elsewhere. The server can always run it.
 */
export function highlighter(): HighlighterTypes {
  return canUseWASM() ? 'highlights' : 'shiki-js';
}

/**
 * The pool that highlights off the main thread, one for the whole page. Vite
 * bundles the worker from this `new URL(...)` in both apps.
 */
export function workerPool(): WorkerPoolManager {
  return getOrCreateWorkerPoolSingleton({
    poolOptions: {
      poolSize: Math.min(3, Math.max(1, (navigator.hardwareConcurrency ?? 2) - 1)),
      // As diffshub: highlights colours a small file on the main thread faster
      // than a worker answers, so the first files show highlighted at once.
      mainThreadMaxCharacters: highlighter() === 'highlights' ? 100_000 : undefined,
      workerFactory: () =>
        new Worker(new URL('@pierre/diffs/worker/worker.js', import.meta.url), { type: 'module' }),
    },
    highlighterOptions: {
      preferredHighlighter: highlighter(),
      theme: CODE_VIEW_OPTIONS.theme,
      langs: ['rust', 'markdown', 'toml', 'typescript', 'yaml', 'sql', 'json', 'sh', 'css'],
    },
  });
}
