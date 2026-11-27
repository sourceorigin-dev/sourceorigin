set working-directory := '..'

# Runs the tests marked as needing TEST_ANALYTICS_URL, whose names say
# `analytics`, in a scratch ClickHouse unless it names one.
# Run the analytics tests against ClickHouse.
[positional-arguments]
test *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -z "${TEST_ANALYTICS_URL:-}" ]]; then exec just analytics scratch just analytics test "$@"; fi
    cargo nextest run --locked --workspace --run-ignored only -E 'test(/analytics/)' "$@"

# TEST_ANALYTICS_URL and TEST_ANALYTICS_PASSWORD name its owner, `sourceorigin`,
# on a free loopback port, and the `sourceorigin` database, which a test's
# `migrate` creates. The container has a name of its own, so worktrees
# never share one. Needs Docker.
# Run a command against a throwaway ClickHouse, e.g. `just analytics scratch -- just coverage`.
[doc("Run a command against a throwaway ClickHouse, e.g. `just analytics scratch -- just coverage`.")]
[positional-arguments]
scratch *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ "${1-}" == -- ]] && shift
    image=$(sed -n 's/^image = "\(clickhouse\/clickhouse-server:[^"]*\)"$/\1/p' infra/piqueld.toml)
    name="sourceorigin-analytics-$$-$RANDOM"
    trap 'docker rm --force "$name" >/dev/null 2>&1 || true' EXIT
    docker run --detach --name "$name" --publish 127.0.0.1::8123 \
        --env CLICKHOUSE_USER=sourceorigin \
        --env CLICKHOUSE_PASSWORD=scratch --env CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1 \
        --volume "$PWD/infra/clickhouse/config.xml:/etc/clickhouse-server/config.d/sourceorigin.xml:ro" \
        "$image" >/dev/null
    port=$(docker port "$name" 8123/tcp | head -n 1 | sed 's/.*://')
    for _ in $(seq 100); do
        curl --silent --fail "http://127.0.0.1:$port/ping" >/dev/null && break
        sleep 0.2
    done
    curl --silent --fail "http://127.0.0.1:$port/ping" >/dev/null || {
        echo "ClickHouse did not start; its log:" >&2
        docker logs "$name" >&2
        exit 1
    }
    export TEST_ANALYTICS_URL="http://sourceorigin@127.0.0.1:$port/sourceorigin" TEST_ANALYTICS_PASSWORD=scratch
    "$@"
