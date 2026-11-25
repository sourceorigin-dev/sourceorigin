import { getLineAnnotationName } from '@pierre/diffs';
import { templateRender } from '@pierre/diffs/react';
import type { ReactNode } from 'react';

import type { Thread } from '@experiment/shared/api';
import { annotationsFor, type ReviewAnnotation } from '@experiment/shared/review';
import type { ServerFile } from '@experiment/shared/markup';

interface ServerFilesProps {
  files: ServerFile[];
  threads: Map<string, Thread[]>;
  renderThread(thread: Thread): ReactNode;
}

/**
 * The files the server rendered, each in a declarative shadow root, with their
 * threads in the slots Pierre's markup leaves for them. They are what a search
 * engine reads and what shows before any script runs; the page removes them
 * once CodeView has rendered.
 */
export function ServerFiles({ files, threads, renderThread }: ServerFilesProps) {
  return (
    <div className="review-diff server-files">
      {files.map((file) => (
        // The browser moves the <template> into a shadow root while parsing, so
        // the element React hydrates no longer has it.
        <diffs-container key={file.path} suppressHydrationWarning>
          {templateRender(
            annotationsFor(threads.get(file.path) ?? []).map((annotation) =>
              annotation.metadata.kind === 'thread' ? (
                <div key={annotation.metadata.thread.id} slot={getLineAnnotationName<ReviewAnnotation>(annotation)}>
                  {renderThread(annotation.metadata.thread)}
                </div>
              ) : null
            ),
            file.html
          )}
        </diffs-container>
      ))}
    </div>
  );
}
