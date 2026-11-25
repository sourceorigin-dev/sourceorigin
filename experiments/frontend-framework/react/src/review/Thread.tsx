import { type FormEvent, useState } from 'react';

import type { Thread } from '@experiment/shared/api';
import { formatTime, initials } from '@experiment/shared/review';

interface CommentFormProps {
  label: string;
  submit: string;
  autoFocus?: boolean;
  onCancel?(): void;
  onSubmit(body: string): Promise<void>;
}

/** The box a comment is written in: a reply, or the first of a new thread. */
function CommentForm({ label, submit, autoFocus, onCancel, onSubmit }: CommentFormProps) {
  const [body, setBody] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function send(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await onSubmit(body);
      setBody('');
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : 'The comment was not saved.');
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={send}>
      <textarea
        aria-label={label}
        placeholder={label}
        rows={2}
        autoFocus={autoFocus}
        value={body}
        onChange={(event) => setBody(event.currentTarget.value)}
      />
      {error != null && <p className="field-error">{error}</p>}
      <div className="actions">
        {onCancel != null && (
          <button type="button" className="secondary small" onClick={onCancel}>
            Cancel
          </button>
        )}
        <button className="small" disabled={busy || body.trim() === ''}>
          {submit}
        </button>
      </div>
    </form>
  );
}

interface ThreadViewProps {
  thread: Thread;
  /** Absent in the files the server rendered, which nobody can reply in yet. */
  onReply?(thread: number, body: string): Promise<void>;
}

/** A thread under a diff line, with the box to reply in. */
export function ThreadView({ thread, onReply }: ThreadViewProps) {
  return (
    <section className="thread" aria-label={`Thread on line ${thread.line}`}>
      {thread.comments.map((comment) => (
        <article className="comment" key={comment.id}>
          <span className="avatar small" aria-hidden="true">
            {initials(comment.author)}
          </span>
          <header>
            <strong>{comment.author}</strong>
            <time dateTime={new Date(comment.created_at * 1000).toISOString()}>
              {formatTime(comment.created_at)}
            </time>
          </header>
          <p>{comment.body}</p>
        </article>
      ))}
      {onReply != null && (
        <CommentForm label="Reply" submit="Reply" onSubmit={(body) => onReply(thread.id, body)} />
      )}
    </section>
  );
}

interface DraftProps {
  onCancel(): void;
  onSubmit(body: string): Promise<void>;
}

/** The thread being started on a line. */
export function Draft({ onCancel, onSubmit }: DraftProps) {
  return (
    <section className="thread" aria-label="New comment">
      <CommentForm label="Comment on this line" submit="Comment" autoFocus onCancel={onCancel} onSubmit={onSubmit} />
    </section>
  );
}
