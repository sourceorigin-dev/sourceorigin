<!-- The box a comment is written in: a reply, or the first of a new thread. -->
<script lang="ts">
  interface Props {
    label: string;
    submit: string;
    autofocus?: boolean;
    oncancel?: () => void;
    onsubmit: (body: string) => Promise<void>;
  }

  let { label, submit, autofocus = false, oncancel, onsubmit }: Props = $props();
  let body = $state('');
  let busy = $state(false);
  let error = $state<string>();

  async function send(event: SubmitEvent) {
    event.preventDefault();
    busy = true;
    error = undefined;
    try {
      await onsubmit(body);
      body = '';
    } catch (cause) {
      error = cause instanceof Error ? cause.message : 'The comment was not saved.';
    } finally {
      busy = false;
    }
  }
</script>

<form onsubmit={send}>
  <!-- svelte-ignore a11y_autofocus -->
  <textarea aria-label={label} placeholder={label} rows="2" {autofocus} bind:value={body}></textarea>
  {#if error}<p class="field-error">{error}</p>{/if}
  <div class="actions">
    {#if oncancel}
      <button type="button" class="secondary small" onclick={oncancel}>Cancel</button>
    {/if}
    <button class="small" disabled={busy || body.trim() === ''}>{submit}</button>
  </div>
</form>
