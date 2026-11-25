//! Reading the analytics database back, as the dashboard's reader, which may
//! only read: what the Analytics page charts, one trace, or a query of one's
//! own. Agents finding patterns use these, with `--json`; see
//! `.claude/skills/sourceorigin-analytics`.

use super::{
    AdminOperation, CliError, Console,
    output::{Cell, HumanWriter, report},
};
use crate::admin::{Analytics, Trace};
use clap::Subcommand;
use serde::Serialize;
use sourceorigin_common::TraceId;
use sourceorigin_telemetry::analytics::Store;
use std::io;

/// What the analytics database holds, read only.
#[derive(Subcommand)]
pub(super) enum AnalyticsCommand {
    /// Print what the dashboard's Analytics page charts: the last 24 hours'
    /// requests, slowest routes, page speed and browser errors, 30 days of
    /// sign-ins, how far sign-ins to apps get, passkeys, and storage.
    Report,
    /// Print one trace's spans, each under the span that started it, and its log
    /// lines.
    Trace {
        /// The trace ID: 32 hex digits, as error pages, the `X-Trace-Id` header
        /// and each event show it.
        id: TraceId,
    },
    /// Run a read-only SQL query on the analytics database and print its rows,
    /// such as `SELECT kind, count() FROM events GROUP BY kind`. ClickHouse
    /// refuses anything but reading, and stops a query past 5 seconds, 512 MiB
    /// or 10,000 rows. Leave out `FORMAT`: rows come back as JSON.
    Query {
        /// The query.
        sql: String,
    },
}

/// A query's rows, as JSON objects.
#[derive(Serialize)]
#[serde(transparent)]
struct Rows(Vec<serde_json::Value>);

impl AnalyticsCommand {
    /// The operation this command runs.
    pub(super) const fn operation(&self) -> AdminOperation {
        match self {
            Self::Report => AdminOperation::ViewAnalytics,
            Self::Trace { .. } => AdminOperation::ViewTrace,
            Self::Query { .. } => AdminOperation::QueryAnalytics,
        }
    }

    /// Runs this command through the console's service, as the reader
    /// `ANALYTICS_READER_URL` names.
    pub(super) async fn run(self, console: &Console) -> Result<(), CliError> {
        let Console {
            service,
            operator,
            out,
            ..
        } = console;
        let reader = Store::reader_from_environment()?.ok_or(CliError::AnalyticsOff)?;
        match self {
            Self::Report => out.emit(&service.analytics(operator, &reader).await?),
            Self::Trace { id } => out.emit(&service.trace(operator, &reader, id).await?),
            Self::Query { sql } => out.emit(&Rows(
                service.query_analytics(operator, &reader, &sql).await?,
            )),
        }
    }
}

report!(Analytics, self, out, {
    self.traffic(out)?;
    self.usage(out)
});

/// The two halves of what the Analytics page charts, at the terminal.
trait AnalyticsTables {
    /// The last 24 hours: requests, the slowest routes, page speed and
    /// browser errors.
    fn traffic(&self, out: &mut HumanWriter<'_>) -> io::Result<()>;
    /// The last 30 days' sign-ins, steps and passkeys, and storage.
    fn usage(&self, out: &mut HumanWriter<'_>) -> io::Result<()>;
}

impl AnalyticsTables for Analytics {
    fn traffic(&self, out: &mut HumanWriter<'_>) -> io::Result<()> {
        out.section("Requests per hour, the last 24, UTC")?;
        out.table(
            ["HOUR", "REQUESTS", "SERVER ERRORS"],
            "No request.",
            self.requests
                .iter()
                .filter(|hour| hour.requests > 0)
                .map(|hour| {
                    [
                        Cell::new(hour.hour),
                        Cell::new(hour.requests),
                        Cell::new(hour.errors),
                    ]
                }),
        )?;
        out.section("Slowest routes, 24 hours")?;
        out.table(
            ["ROUTE", "SERVER", "REQUESTS", "MEDIAN MS", "P95 MS"],
            "No request.",
            self.slowest.iter().map(|route| {
                [
                    Cell::new(format_args!("{} {}", route.method, route.route)),
                    Cell::new(&route.service),
                    Cell::new(route.requests),
                    Cell::new(format_args!("{:.1}", route.median_ms)),
                    Cell::new(format_args!("{:.1}", route.p95_ms)),
                ]
            }),
        )?;
        out.section("Page speed, 24 hours, p75")?;
        let shown = |value: Option<f64>| value.map(|value| format!("{value:.0}")).into();
        out.table(
            ["PAGE", "VIEWS", "LCP MS", "INP MS", "CLS"],
            "No page measured.",
            self.pages.iter().map(|page| {
                [
                    Cell::new(&page.route),
                    Cell::new(page.views),
                    shown(page.lcp_ms),
                    shown(page.inp_ms),
                    page.cls.map(|cls| format!("{cls:.2}")).into(),
                ]
            }),
        )?;
        out.section("Browser errors, 24 hours")?;
        out.table(
            ["ERROR", "SCRIPT", "COUNT"],
            "No script error.",
            self.browser_errors.iter().map(|error| {
                [
                    Cell::new(&error.name),
                    Cell::new(&error.script),
                    Cell::new(error.count),
                ]
            }),
        )?;
        Ok(())
    }

    fn usage(&self, out: &mut HumanWriter<'_>) -> io::Result<()> {
        out.section("Sign-ins per day, the last 30, UTC")?;
        out.table(
            ["DAY", "SIGNED IN", "FAILED"],
            "No sign-in.",
            self.sign_ins
                .iter()
                .filter(|day| day.signed_in + day.failed > 0)
                .map(|day| {
                    [
                        Cell::new(day.day),
                        Cell::new(day.signed_in),
                        Cell::new(day.failed),
                    ]
                }),
        )?;
        out.section("Signing in to apps, 30 days")?;
        out.table(
            ["STEP", "COUNT"],
            "No step.",
            self.steps
                .iter()
                .map(|(step, count)| [Cell::new(step), Cell::new(count)]),
        )?;
        out.section("Passkeys used, 30 days")?;
        out.table(
            ["SYNCED", "TRANSPORT", "COUNT"],
            "No passkey use.",
            self.passkeys.iter().map(|kind| {
                [
                    Cell::new(kind.synced),
                    Cell::new(kind.transport),
                    Cell::new(kind.count),
                ]
            }),
        )?;
        out.section("Storage")?;
        out.table(
            ["TABLE", "ROWS", "BYTES"],
            "No table.",
            self.tables.iter().map(|table| {
                [
                    Cell::new(&table.table),
                    Cell::new(table.rows),
                    Cell::new(table.bytes),
                ]
            }),
        )
    }
}

report!(Trace, self, out, {
    out.section("Spans")?;
    out.table(
        ["SPAN", "SERVICE", "STATUS", "DURATION US"],
        "No span.",
        self.spans.iter().map(|(depth, span)| {
            [
                Cell::new(format_args!("{}{}", "  ".repeat(*depth), span.name)),
                Cell::new(&span.service),
                Cell::new(&span.status),
                Cell::new(span.duration_us),
            ]
        }),
    )?;
    out.section("Log lines")?;
    out.table(
        ["LEVEL", "MODULE", "MESSAGE"],
        "No log line.",
        self.logs.iter().map(|line| {
            [
                Cell::new(&line.level),
                Cell::new(&line.target),
                Cell::new(&line.body),
            ]
        }),
    )?;
    if self.more_spans || self.more_logs {
        out.line(format_args!(
            "The trace has more: the first {} spans and log lines are shown.",
            crate::admin::SHOWN
        ))?;
    }
    Ok(())
});

report!(Rows, self, out, {
    if self.0.is_empty() {
        return out.line("No rows.");
    }
    self.0.iter().try_for_each(|row| out.line(row))
});
