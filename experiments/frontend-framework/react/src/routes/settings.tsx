import { AlertDialog } from '@base-ui/react/alert-dialog';
import { Field } from '@base-ui/react/field';
import { Form } from '@base-ui/react/form';
import { createFileRoute, Link } from '@tanstack/react-router';
import { type FormEvent, useState } from 'react';

import { getAccount, revokeOtherSessions, updateAccount } from '@experiment/shared/api';

export const Route = createFileRoute('/settings')({
  // Runs on the server for the first request and in the browser after that:
  // the generated client is the same in both.
  loader: async () => (await getAccount({ throwOnError: true })).data,
  head: () => ({ meta: [{ title: 'Account settings · SourceOrigin' }] }),
  component: Settings,
});

function Settings() {
  // Kept apart from useState on purpose: the hook's type follows the place it is
  // used, so `useState(Route.useLoaderData())` is not checked against the loader.
  const loaded = Route.useLoaderData();
  const [account, setAccount] = useState(loaded);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<'idle' | 'saving' | 'saved'>('idle');

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    setStatus('saving');
    const result = await updateAccount({
      body: {
        display_name: String(form.get('display_name')),
        email: String(form.get('email')),
        bio: String(form.get('bio')),
        notify_on_review: form.get('notify_on_review') === 'on',
      },
    });
    if (result.error != null) {
      // The API names each field it refused; Base UI's Form shows each under its field.
      setErrors(result.error.fields);
      setStatus('idle');
      return;
    }
    setErrors({});
    setAccount(result.data);
    setStatus('saved');
  }

  async function signOutOthers() {
    const result = await revokeOtherSessions();
    if (result.data != null) setAccount(result.data);
  }

  const others = account.other_sessions;
  return (
    <main className="settings">
      <nav className="trail" aria-label="Breadcrumb">
        <Link to="/">SourceOrigin</Link>
        <span aria-hidden="true">/</span>
        <span>Account settings</span>
      </nav>
      <h1>Account settings</h1>

      <section className="card" aria-labelledby="profile">
        <header>
          <h2 id="profile">Profile</h2>
        </header>
        <Form errors={errors} onSubmit={save}>
          <Field.Root name="display_name" className="field">
            <Field.Label>Display name</Field.Label>
            <Field.Control required defaultValue={account.display_name} />
            <Field.Description className="field-help">Shown beside your comments.</Field.Description>
            <Field.Error className="field-error" />
          </Field.Root>
          <Field.Root name="email" className="field">
            <Field.Label>Email</Field.Label>
            <Field.Control type="email" required defaultValue={account.email} />
            <Field.Error className="field-error" />
          </Field.Root>
          <Field.Root name="bio" className="field">
            <Field.Label>About you</Field.Label>
            <Field.Control render={<textarea rows={3} />} defaultValue={account.bio} />
            <Field.Error className="field-error" />
          </Field.Root>
          <label className="check">
            <input type="checkbox" name="notify_on_review" defaultChecked={account.notify_on_review} />
            Email me when someone reviews my change
          </label>
          <div className="row">
            <button disabled={status === 'saving'}>Save settings</button>
            <span role="status" className="muted">
              {status === 'saved' ? 'Settings saved.' : ''}
            </span>
          </div>
        </Form>
      </section>

      <section className="card" aria-labelledby="sessions">
        <header>
          <h2 id="sessions">Sessions</h2>
        </header>
        <div className="body">
          <p>
            {others === 0
              ? 'No other browser is signed in to your Origin Account.'
              : `${others} other browsers are signed in to your Origin Account.`}
          </p>
          <div>
          <AlertDialog.Root>
            <AlertDialog.Trigger className="danger" disabled={others === 0}>
              Sign out other browsers
            </AlertDialog.Trigger>
            <AlertDialog.Portal>
              <AlertDialog.Backdrop className="dialog-backdrop" />
              <AlertDialog.Popup className="dialog modal">
                <AlertDialog.Title>Sign out {others} other browsers?</AlertDialog.Title>
                <AlertDialog.Description>
                  Each of them has to sign in again. This browser stays signed in.
                </AlertDialog.Description>
                <div className="row">
                  <AlertDialog.Close className="danger" onClick={signOutOthers}>
                    Sign out other browsers
                  </AlertDialog.Close>
                  <AlertDialog.Close className="secondary">Keep them signed in</AlertDialog.Close>
                </div>
              </AlertDialog.Popup>
            </AlertDialog.Portal>
          </AlertDialog.Root>
          </div>
        </div>
      </section>
    </main>
  );
}
