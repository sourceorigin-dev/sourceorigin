//! Broken telemetry stops only `serve`: every other command, such as a migration
//! or a break-glass repair, warns and runs with logs on stderr only.

use std::process::{Command, Output};

/// Runs `auth-service` with `args`, and nothing in its environment but a
/// malformed analytics database URL, which telemetry is exported to.
fn run(args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_auth-service"))
        .args(args)
        .env_clear()
        .env("ANALYTICS_URL", "not a url")
        .env("ANALYTICS_PASSWORD", "env://ANALYTICS_PASSWORD_VALUE")
        .env("ANALYTICS_PASSWORD_VALUE", "password")
        .output()
}

/// `migrate` warns, twice, then goes on to its own work, which fails only for
/// want of a database.
#[test]
fn commands_run_when_export_cannot_start() {
    let output = run(&["migrate"]).expect("auth-service runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("telemetry export could not start; logging to stderr only"),
        "{stderr}"
    );
    assert!(
        stderr.contains("analytics tables not migrated: ANALYTICS_URL must be"),
        "{stderr}"
    );
    assert!(
        stderr.contains("Error: AUTH_DATABASE_URL is required"),
        "{stderr}"
    );
}

/// `serve` refuses to start without the telemetry its alerts read.
#[test]
fn serve_fails_when_export_cannot_start() {
    let output = run(&["serve"]).expect("auth-service runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Error: ANALYTICS_URL must be"), "{stderr}");
}
