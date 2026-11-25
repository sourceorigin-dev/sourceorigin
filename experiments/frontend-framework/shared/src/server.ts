// What the server does for the first screen, the same in both candidates: ask
// the API for the change, and render with Pierre's server entry points what
// they can render without a browser. CodeView is not among them: it builds DOM
// in its constructor, so the server renders the first files one by one and
// CodeView takes over in the browser.

import { preloadDiffHTML } from '@pierre/diffs/ssr';
import { preloadFileTree } from '@pierre/trees/ssr';

import { type ChangeDetail, getChange, getPatch, listThreads, type Thread } from './api';
import {
  annotationsFor,
  CODE_VIEW_OPTIONS,
  itemFromPatch,
  SERVER_RENDERED_FILES,
  splitPatch,
  threadsByPath,
  treeOptions,
} from './review';

/** One file the server rendered: its path, and the markup of its shadow root. */
export interface ServerFile {
  path: string;
  html: string;
}

/** Everything the review page needs before the first paint. */
export interface FirstScreen {
  change: ChangeDetail;
  threads: Thread[];
  /** The patch of the first files, so CodeView shows them without a request. */
  firstPatch: string;
  serverFiles: ServerFile[];
  tree: { id: string; shadowHtml: string };
}

export async function loadFirstScreen(change: string): Promise<FirstScreen> {
  const path = { change };
  const [detail, threads, patch] = await Promise.all([
    getChange({ path, throwOnError: true }),
    listThreads({ path, throwOnError: true }),
    getPatch({ path, query: { limit: SERVER_RENDERED_FILES }, parseAs: 'text', throwOnError: true }),
  ]);
  const byPath = threadsByPath(threads.data);
  const firstPatch = patch.data as string;

  const serverFiles: ServerFile[] = [];
  for (const text of splitPatch(firstPatch)) {
    const item = itemFromPatch(text);
    if (item == null) continue;
    const annotations = annotationsFor(byPath.get(item.id) ?? []);
    const { layout: _layout, stickyHeaders: _sticky, ...options } = CODE_VIEW_OPTIONS;
    serverFiles.push({
      path: item.id,
      html: await preloadDiffHTML({ fileDiff: item.fileDiff, annotations, options }),
    });
  }

  const tree = preloadFileTree(treeOptions(detail.data.files));
  return {
    change: detail.data,
    threads: threads.data,
    firstPatch,
    serverFiles,
    tree: { id: tree.id, shadowHtml: tree.shadowHtml },
  };
}
