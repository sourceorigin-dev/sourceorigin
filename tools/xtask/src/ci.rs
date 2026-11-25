//! The jobs of .github/workflows/ci.yml, as `just check-changed` runs them.
//! The `ci_matches_the_workflow` test fails until this table and the workflow
//! change together.

use sourceorigin_standard::check_changed::{self, Ci, Job, Recipe, Run, Trigger};

/// A filter of .github/filters.yml.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Filter {
    /// What the Rust crates are built and tested from.
    Rust,
    /// The generated reference pages auth's tests compare with the code.
    Reference,
    /// Every file Prettier, `ESLint` or `tsc` can see.
    Web,
    /// The Rust and npm dependency manifests and lockfiles, and `deny.toml`.
    Deps,
    /// What the running service, its images and the browser suite are built from.
    App,
    /// The piqueld manifests and the telemetry configs.
    Infra,
}

impl check_changed::Filter for Filter {
    const ALL: &'static [Self] = &[
        Self::Rust,
        Self::Reference,
        Self::Web,
        Self::Deps,
        Self::App,
        Self::Infra,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Reference => "reference",
            Self::Web => "web",
            Self::Deps => "deps",
            Self::App => "app",
            Self::Infra => "infra",
        }
    }
}

/// A recipe of the justfile, run here on the whole workspace.
const fn here(command: &'static [&'static str]) -> Recipe {
    Recipe {
        command,
        run: Run::Workspace,
    }
}

/// A recipe of the justfile that only CI runs, for `reason`.
const fn in_ci(command: &'static [&'static str], reason: &'static str) -> Recipe {
    Recipe {
        command,
        run: Run::InCi(reason),
    }
}

/// What the `rust` output of the `changes` job is built from.
const RUST: &[Filter] = &[Filter::Rust, Filter::Reference];

/// Why the browser suite's steps run only in CI.
const E2E: &str = "needs the built service behind Caddy, with PostgreSQL and Mailpit";

/// CI's jobs, in the workflow's order. `docker` runs no recipe, and `ci` only
/// sums up the others.
pub(crate) const CI: Ci<Filter> = Ci {
    jobs: &[
        Job {
            id: "checks",
            trigger: Trigger::AnyOf(&[Filter::Rust, Filter::Reference, Filter::Deps]),
            recipes: &[
                here(&["just", "rust", "fmt-check"]),
                here(&["just", "rust", "unused"]),
                here(&["just", "rust", "audit"]),
            ],
        },
        Job {
            id: "web",
            trigger: Trigger::AnyOf(&[Filter::Web, Filter::Deps]),
            recipes: &[
                here(&["just", "install"]),
                here(&["just", "web", "fmt-check"]),
                here(&["just", "web", "check"]),
                here(&["just", "web", "lint"]),
                here(&["just", "web", "unused"]),
                here(&["just", "web", "test"]),
                in_ci(&["just", "web", "smoke"], "needs Bun and workerd"),
                here(&["just", "web", "audit"]),
            ],
        },
        Job {
            id: "rust-lint",
            trigger: Trigger::AnyOf(RUST),
            recipes: &[
                here(&["just", "rust", "lint"]),
                here(&["just", "doc"]),
                here(&["just", "doc-test"]),
            ],
        },
        Job {
            id: "package",
            trigger: Trigger::AnyOf(&[Filter::Rust, Filter::Reference, Filter::Web]),
            recipes: &[
                here(&["just", "install"]),
                in_ci(
                    &["just", "publish", "--dry-run"],
                    "slow: packages every SDK and builds it from its package",
                ),
            ],
        },
        Job {
            id: "msrv",
            trigger: Trigger::AnyOf(RUST),
            recipes: &[in_ci(
                &["just", "rust", "features"],
                "needs the oldest supported Rust",
            )],
        },
        Job {
            id: "rust",
            trigger: Trigger::AnyOf(RUST),
            recipes: &[
                in_ci(&["just", "db", "install-sqlx"], "installs sqlx-cli"),
                in_ci(
                    &["just", "coverage"],
                    "slow: every test, instrumented, on a scratch cluster",
                ),
                in_ci(
                    &["just", "db", "prepare-check"],
                    "needs the database the coverage run migrated",
                ),
                in_ci(&["just", "coverage-build"], "main builds its cache with it"),
                in_ci(&["just", "db", "check"], "main builds its cache with it"),
            ],
        },
        Job {
            id: "e2e",
            trigger: Trigger::AnyOf(&[Filter::App]),
            recipes: &[
                here(&["just", "install"]),
                in_ci(&["just", "rust", "build"], E2E),
                in_ci(&["just", "auth", "generate-secret"], E2E),
                in_ci(&["just", "web", "browser-client"], E2E),
                in_ci(&["just", "web", "test-browser"], E2E),
            ],
        },
        Job {
            id: "infra",
            trigger: Trigger::AnyOf(&[Filter::Infra]),
            recipes: &[in_ci(
                &["just", "infra", "check"],
                "needs piquelctl and Docker",
            )],
        },
        Job {
            id: "standard",
            trigger: Trigger::EveryChange,
            recipes: &[here(&["just", "standard"])],
        },
        Job {
            id: "docs",
            trigger: Trigger::EveryChange,
            recipes: &[here(&["just", "docs"])],
        },
    ],
    ignored: &["docker", "ci"],
};

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// Fails until the table, ci.yml and filters.yml agree.
    #[test]
    fn ci_matches_the_workflow() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
        super::CI
            .check_workflow(root)
            .expect("the CI table matches ci.yml and filters.yml");
    }
}
