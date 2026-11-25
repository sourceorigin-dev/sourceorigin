<!-- A thread under a diff line, with the box to reply in. -->
<script lang="ts">
  import type { Thread } from '@experiment/shared/api';
  import { formatTime, initials } from '@experiment/shared/review';

  import CommentForm from './CommentForm.svelte';

  interface Props {
    thread: Thread;
    /** Absent in the files the server rendered, which nobody can reply in yet. */
    onreply?: (thread: number, body: string) => Promise<void>;
  }

  let { thread, onreply }: Props = $props();
</script>

<section class="thread" aria-label="Thread on line {thread.line}">
  {#each thread.comments as comment (comment.id)}
    <article class="comment">
      <span class="avatar small" aria-hidden="true">{initials(comment.author)}</span>
      <header>
        <strong>{comment.author}</strong>
        <time datetime={new Date(comment.created_at * 1000).toISOString()}>
          {formatTime(comment.created_at)}
        </time>
      </header>
      <p>{comment.body}</p>
    </article>
  {/each}
  {#if onreply}
    <CommentForm label="Reply" submit="Reply" onsubmit={(body) => onreply(thread.id, body)} />
  {/if}
</section>
