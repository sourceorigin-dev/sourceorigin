# Usage statistics

SourceOrigin counts how Origin Auth is used, such as which kinds of passkeys
people sign in with, to see what works and what fails. The counts never say
who you are. You can leave yours out at any time.

## What is counted

- Whether the passkey you sign in or confirm with is synced between devices,
  as a password manager's or a phone's are, or kept on one device, as a
  security key's is, and how your browser reaches it: built in, on a phone, or
  over USB, NFC or Bluetooth.
- How far each sign-in to an app gets: whether the app sent you here, the
  sign-in or consent page showed, and whether the app got access or you
  declined. These are counted for the app, so its developer can see where
  people stop.

Each count holds only that kind, and the day it happened, not the time. It
doesn't hold your account, your name, your email address or your IP address.
Counts are kept up to 13 months.

SourceOrigin also keeps service logs and counts that keep it running, such as
how long pages take and how many sign-ins fail. Those are part of running the
service, so this setting doesn't turn them off. The
[privacy policy](https://sourceorigin.dev/privacy) lists everything kept, and
for how long.

## Leave usage statistics

1. In your settings, open **Your data**.
2. Under **Usage statistics**, turn off **Include my activity in usage
   statistics**.

From then on, nothing you do is counted in usage statistics. Turn it back on the
same way. Each change is recorded in your **Security history**, as "Usage
statistics turned off" or "Usage statistics turned on".

## Global Privacy Control

If your browser sends [Global Privacy Control](https://globalprivacycontrol.org),
a setting some browsers and extensions offer to ask sites not to track you,
nothing you do in that browser is counted in usage statistics, whatever this
setting says.

## Check that it works

**Your data** shows the switch off, and your
[data download](export-your-data.md) gives when you left, as
`left_usage_statistics_at` in your profile.
