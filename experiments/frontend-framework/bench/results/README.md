# What was recorded

Times are milliseconds unless a name says otherwise; `cpu` figures are CPU time
of the page's main thread. Each run alternates the two apps.

| File | What it is |
| --- | --- |
| `calm-run.txt` | The full run on a calm machine, load average about 4. The report's timings are from it |
| `measure-busy-machine.json` | The same full run later, of the final code, while other programs compiled, load average 7 to 10. The report's byte counts are from it |
| `measure-review-third.json` | A third reading of the review page alone, load average 3 rising to 9 |
| `measure-review-shiki-js.json` | The review page before WebAssembly was allowed, so with Shiki's JavaScript engine |
| `measure-shiki-js.json` | A full run of that earlier state, on a busy machine, load average 8 to 24 |
| `devloop.json` | Cold start and edit latency of the two dev servers |
