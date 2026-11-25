# The frontend framework experiment

The same two screens, change review and account settings, built in React with
TanStack Start and in SvelteKit, to choose SourceOrigin's frontend framework
(issue #774). [REPORT.md](REPORT.md) has the measurements and the
recommendation.

This code is thrown away after the decision. It lives on a branch that never
merges, and it does not follow the repository's linters or its engineering
standard.

## What is here

| Folder | What it holds |
| --- | --- |
| `api` | The stub API both apps talk to, in Rust with axum and utoipa. In memory, restarts clean |
| `openapi.json` | The API's description, printed by `stub-api --openapi` |
| `shared` | What both apps share and no framework is in: the generated API client, the helpers around Pierre's data, what the server renders, the Content Security Policy and the stylesheet |
| `react` | React 19 with TanStack Start 1.169, Base UI for the form and the dialog |
| `svelte` | Svelte 5 with SvelteKit 3.4, Bits UI for the dialog |
| `bench` | One walk through both apps, and the measurements. `bench/results` keeps what was recorded |
| `fixture` | A script that writes the fixture diff from this repository's own history |

The fixture is everything that landed on `main` between `1f2f4a231` and
`386525cd2`: 755 files, 21,210 added and 14,021 removed lines, a 2.5 MB patch.

## Run it

Everything listens on `127.0.0.1`, so use a browser on the same machine or
forward the ports. Nothing here starts by itself.

```console
cd experiments/frontend-framework
./fixture/generate.sh                      # needs a full clone
(cd api && cargo build --release)
pnpm install
./bench/api.sh                             # the stub API, on port 4100

(cd react && pnpm build && node server.js)    # http://127.0.0.1:4301
(cd svelte && pnpm build && node server.js)   # http://127.0.0.1:4302
```

Each app has `/changes/1` and `/settings`. The review page's two "Simulate"
buttons make the stub API send an update as another person would.

For the development servers, `pnpm dev` in `react` (port 4201) or `svelte`
(port 4202).

After the API changes: `api/target/release/stub-api --openapi > openapi.json`,
then `pnpm generate` in `shared`.

## Check and measure

```console
./bench/api.sh && node bench/check.mjs http://127.0.0.1:4301   # the walk, 15 steps
./bench/api.sh && node bench/check.mjs http://127.0.0.1:4302
node bench/measure.mjs 9          # both production builds, taking turns
node bench/devloop.mjs            # cold start and edit latency; starts its own dev servers
./bench/builds.sh                 # build and type-check times
./bench/lines.sh                  # lines of each part
node bench/wasm.mjs http://127.0.0.1:4301    # whether the policy lets WebAssembly run
node bench/bytes.mjs http://127.0.0.1:4301/settings   # the scripts one page loads
```

`bench/api.sh` restarts the stub API, so every walk starts from the same state.
