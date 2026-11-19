<!-- Title: a conventional commit that says what changes for whoever uses it, such as
`fix(auth): app logout ends inline sessions and never blocks`. Keep the pull
request to one change; stack dependent ones. -->

## What and why

<!-- What was wrong, what this changes, and the issue it closes ("Closes #123"). -->

## Testing

<!-- How you tested it, including what you didn't test (real devices, live providers). -->

## Checklist

- [ ] Every surface gets the feature: the CLI, settings, developer console, admin dashboard, SDKs and documentation.
- [ ] `just validate` passes, plus `just db test` or the browser suite where the change can affect them.
- [ ] Generated files are regenerated in the same commit.
