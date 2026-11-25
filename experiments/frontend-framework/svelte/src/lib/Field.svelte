<!--
  A form field: its label, its help and its error, tied to the control with the
  ARIA attributes a screen reader needs. Bits UI has no field component, so
  this is ours; Base UI ships the React one.
-->
<script lang="ts">
  import type { Snippet } from 'svelte';

  /** What the control must carry to belong to this field. */
  interface ControlAttributes {
    id: string;
    name: string;
    'aria-invalid'?: 'true';
    'aria-describedby'?: string;
  }

  interface Props {
    label: string;
    name: string;
    help?: string;
    /** Why the server refused the value, if it did. */
    error?: string;
    control: Snippet<[ControlAttributes]>;
  }

  let { label, name, help, error, control }: Props = $props();
  const id = $props.id();
  const described = $derived(
    [help && `${id}-help`, error && `${id}-error`].filter(Boolean).join(' ') || undefined
  );
</script>

<div class="field">
  <label for={id}>{label}</label>
  {@render control({
    id,
    name,
    'aria-invalid': error ? 'true' : undefined,
    'aria-describedby': described,
  })}
  {#if help}<p id="{id}-help" class="field-help">{help}</p>{/if}
  {#if error}<p id="{id}-error" class="field-error">{error}</p>{/if}
</div>
