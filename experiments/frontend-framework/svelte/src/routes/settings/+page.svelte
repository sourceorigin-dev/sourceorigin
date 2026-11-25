<script lang="ts">
  import { AlertDialog } from 'bits-ui';

  import Field from '#lib/Field.svelte';
  import { revokeOtherSessions, updateAccount } from '@experiment/shared/api';

  import type { PageProps } from './$types';

  let { data }: PageProps = $props();
  // Starts as what the server loaded, then follows what the API answers.
  let account = $derived(data.account);
  let errors = $state<Record<string, string>>({});
  let status = $state<'idle' | 'saving' | 'saved'>('idle');
  let confirming = $state(false);

  async function save(event: SubmitEvent & { currentTarget: HTMLFormElement }) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    status = 'saving';
    const result = await updateAccount({
      body: {
        display_name: String(form.get('display_name')),
        email: String(form.get('email')),
        bio: String(form.get('bio')),
        notify_on_review: form.get('notify_on_review') === 'on',
      },
    });
    if (result.error != null) {
      // The API names each field it refused; each Field shows its own.
      errors = result.error.fields;
      status = 'idle';
      return;
    }
    errors = {};
    account = result.data;
    status = 'saved';
  }

  async function signOutOthers() {
    const result = await revokeOtherSessions();
    if (result.data != null) account = result.data;
    confirming = false;
  }

  const others = $derived(account.other_sessions);
</script>

<svelte:head><title>Account settings · SourceOrigin</title></svelte:head>

<main class="settings">
  <nav class="trail" aria-label="Breadcrumb">
    <a href="/">SourceOrigin</a>
    <span aria-hidden="true">/</span>
    <span>Account settings</span>
  </nav>
  <h1>Account settings</h1>

  <section class="card" aria-labelledby="profile">
    <header>
      <h2 id="profile">Profile</h2>
    </header>
    <form onsubmit={save}>
      <Field
        label="Display name"
        name="display_name"
        help="Shown beside your comments."
        error={errors.display_name}
      >
        {#snippet control(attributes)}
          <input {...attributes} required value={account.display_name} />
        {/snippet}
      </Field>
      <Field label="Email" name="email" error={errors.email}>
        {#snippet control(attributes)}
          <input {...attributes} type="email" required value={account.email} />
        {/snippet}
      </Field>
      <Field label="About you" name="bio" error={errors.bio}>
        {#snippet control(attributes)}
          <textarea {...attributes} rows="3" value={account.bio}></textarea>
        {/snippet}
      </Field>
      <label class="check">
        <input type="checkbox" name="notify_on_review" checked={account.notify_on_review} />
        Email me when someone reviews my change
      </label>
      <div class="row">
        <button disabled={status === 'saving'}>Save settings</button>
        <span role="status" class="muted">{status === 'saved' ? 'Settings saved.' : ''}</span>
      </div>
    </form>
  </section>

  <section class="card" aria-labelledby="sessions">
    <header>
      <h2 id="sessions">Sessions</h2>
    </header>
    <div class="body">
      <p>
        {others === 0
          ? 'No other browser is signed in to your Origin Account.'
          : `${others} other browsers are signed in to your Origin Account.`}
      </p>
      <div>
        <AlertDialog.Root bind:open={confirming}>
          <AlertDialog.Trigger class="danger" disabled={others === 0}>
            Sign out other browsers
          </AlertDialog.Trigger>
          <AlertDialog.Portal>
            <AlertDialog.Overlay class="dialog-backdrop" />
            <AlertDialog.Content class="dialog modal">
              <AlertDialog.Title level={2}>Sign out {others} other browsers?</AlertDialog.Title>
              <AlertDialog.Description>
                Each of them has to sign in again. This browser stays signed in.
              </AlertDialog.Description>
              <div class="row">
                <AlertDialog.Action class="danger" onclick={signOutOthers}>
                  Sign out other browsers
                </AlertDialog.Action>
                <AlertDialog.Cancel class="secondary">Keep them signed in</AlertDialog.Cancel>
              </div>
            </AlertDialog.Content>
          </AlertDialog.Portal>
        </AlertDialog.Root>
      </div>
    </div>
  </section>
</main>
