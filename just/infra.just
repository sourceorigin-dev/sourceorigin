set working-directory := '..'

# `--slot NAME` keeps several previews of one branch. The branch is read before
# deploying, so switching branch during the deploy doesn't change the preview
# it waits for.
# Deploy this worktree, uncommitted changes included, to its preview; print its URLs.
preview *args:
    #!/usr/bin/env bash
    set -euo pipefail
    branch=$(git branch --show-current)
    piquelctl --timeout 30m preview deploy sourceorigin --local {{ args }}
    piquelctl --timeout 10m preview wait sourceorigin "$branch" {{ args }} --ready routes

# Delete this worktree's preview with its database.
preview-delete *args:
    #!/usr/bin/env bash
    set -euo pipefail
    branch=$(git branch --show-current)
    piquelctl --timeout 5m preview delete sourceorigin "$branch" {{ args }} --yes

# ClickHouse's config is checked by the image infra/piqueld.toml runs it with,
# so by the version that will read it. Needs piquelctl and Docker.
# Validate the piqueld manifest and ClickHouse's config.
check:
    #!/usr/bin/env bash
    set -euo pipefail
    piquelctl app validate --file infra/piqueld.toml
    clickhouse=$(grep -o 'image = "clickhouse/clickhouse-server:[^"]*"' infra/piqueld.toml | cut -d '"' -f 2)
    docker run --rm --volume "$PWD/infra/clickhouse/config.xml:/etc/clickhouse-server/config.d/sourceorigin.xml:ro" \
        --entrypoint clickhouse "$clickhouse" extract-from-config \
        --config-file /etc/clickhouse-server/config.xml --key=max_server_memory_usage_to_ram_ratio >/dev/null
