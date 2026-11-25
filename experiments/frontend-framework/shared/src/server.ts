// What the server does for the first screen, the same in both candidates: ask
// the API for the change, and render with Pierre's server entry points what
// they can render without a browser. CodeView is not among them: it builds DOM
// in its constructor, so the server renders the first files one by one and
// CodeView takes over in the browser.

import { preloadDiffHTML } from '@pierre/diffs/ssr';
import { preloadFileTree } from '@pierre/trees/ssr';

import { type ChangeDetail, getChange, getPatch, listThreads, type Thread } from './api';
import { keepServerMarkup, type ServerFile } from './markup';
import {
  annotationsFor,
  CODE_VIEW_OPTIONS,
  highlighter,
  itemFromPatch,
  SERVER_RENDERED_FILES,
  splitPatch,
  threadsByPath,
  treeOptions,
} from './review';

/** Everything the review page needs before the first paint. */
export interface FirstScreen {
  change: ChangeDetail;
  threads: Thread[];
  /** The patch of the first files, so CodeView shows them without a request. */
  firstPatch: string;
  /** The id the server gave the file tree, which the browser's tree must share. */
  treeId: string;
}

/**
 * Loads the first screen. What Pierre rendered is not in the answer: it is
 * filed under the answer's `change`, for `serverMarkup` to find while the
 * server renders the page.
 */
export async function loadFirstScreen(change: string): Promise<FirstScreen> {
  const path = { change };
  const [detail, threads, patch] = await Promise.all([
    getChange({ path, throwOnError: true }),
    listThreads({ path, throwOnError: true }),
    getPatch({ path, query: { limit: SERVER_RENDERED_FILES }, parseAs: 'text', throwOnError: true }),
  ]);
  const byPath = threadsByPath(threads.data);
  const firstPatch = patch.data as string;

  const files: ServerFile[] = [];
  for (const text of splitPatch(firstPatch)) {
    const item = itemFromPatch(text);
    if (item == null) continue;
    const annotations = annotationsFor(byPath.get(item.id) ?? []);
    const { layout: _layout, stickyHeaders: _sticky, ...shared } = CODE_VIEW_OPTIONS;
    const options = { ...shared, preferredHighlighter: highlighter() };
    files.push({
      path: item.id,
      html: await preloadDiffHTML({ fileDiff: item.fileDiff, annotations, options }),
    });
  }

  const tree = preloadFileTree(treeOptions(detail.data.files));
  keepServerMarkup(detail.data, { files, tree: tree.shadowHtml });
  return { change: detail.data, threads: threads.data, firstPatch, treeId: tree.id };
}
