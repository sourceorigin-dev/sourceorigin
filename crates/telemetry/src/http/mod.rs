//! One server span, access log line and `requests` row per request.

use crate::{
    Telemetry,
    analytics::{Instance, Route, Signal},
};
use axum::{
    Router,
    extract::{MatchedPath, Request, State},
    http::{HeaderName, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::Response,
};
use opentelemetry::KeyValue;
use sourceorigin_common::TraceId;
use std::{sync::OnceLock, time::Instant};
use tracing::{Instrument, field::Empty};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Methods the conventions name; any other is `_OTHER`, so methods stay bounded.
/// A `static`, so the names it lends are `'static`.
static METHODS: [Method; 9] = [
    Method::GET,
    Method::HEAD,
    Method::POST,
    Method::PUT,
    Method::DELETE,
    Method::CONNECT,
    Method::OPTIONS,
    Method::TRACE,
    Method::PATCH,
];

/// Stands for the path of a request that matched no route, in its access log
/// line. The path itself is never recorded: it is whatever the client sent, such
/// as a mistyped link with an address in it.
const UNMATCHED: &str = "unmatched";

/// The service [`Telemetry::init`] installed, which every `requests` row names.
pub(crate) static SERVICE: OnceLock<&'static str> = OnceLock::new();

crate::signal! {
    /// One request a service answered, once its response head was ready.
    pub struct RequestServed in "requests", kept 30 days, ordered by (service, status) {
        /// The service that answered, such as `auth-service`.
        pub service: &'static str,
        /// The process that answered.
        pub instance: Instance,
        /// Its listener, when it has several, such as `admin`.
        pub listener: Option<&'static str>,
        /// The method, or `_OTHER` for one HTTP's conventions don't name.
        pub method: &'static str,
        /// The route template it matched, or none.
        pub route: Option<Route>,
        /// The response's status code.
        pub status: u16,
        /// How long it took, in microseconds.
        pub duration_us: u64,
        /// Its trace.
        pub trace: Option<TraceId>,
    }
}

/// Traces, records and logs every request of a router. Wrap the whole router,
/// outside any layer that can reject a request, so rejections count too:
/// `HttpTelemetry::default().quiet(&["/health"]).wrap(router)`.
///
/// Each request gets a root span named after its method and route template, with
/// the HTTP semantic conventions' `http.request.method`, `http.route` (absent when
/// no route matched) and `http.response.status_code`. Paths, query strings and
/// headers are never recorded. A 5xx response marks the span failed and
/// carries its trace ID in [`Self::TRACE_ID_HEADER`]. Each request is also a
/// [`RequestServed`] row.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpTelemetry {
    /// Routes whose access log lines are debug output only, such as health probes.
    quiet: &'static [&'static str],
    /// Names the listener when a service has several, as `sourceorigin.listener`.
    listener: Option<&'static str>,
}

impl HttpTelemetry {
    /// Response header with the trace ID of a failed request, so a report can be
    /// matched to its trace. Browsers read it only where CORS exposes it.
    pub const TRACE_ID_HEADER: HeaderName = HeaderName::from_static("x-trace-id");

    /// The trace ID that a response at `status`, answered from within its request,
    /// carries in [`Self::TRACE_ID_HEADER`]: a server error's, and no other. A page
    /// shows it too, since a browser never shows the header.
    #[must_use]
    pub fn trace_id(status: StatusCode) -> Option<TraceId> {
        status
            .is_server_error()
            .then(Telemetry::current_trace_id)
            .flatten()
    }

    /// Logs requests to `routes` at debug level only, and records no row of
    /// them; they are still traced.
    #[must_use]
    pub const fn quiet(self, routes: &'static [&'static str]) -> Self {
        Self {
            quiet: routes,
            ..self
        }
    }

    /// Labels the requests' spans and log lines with `sourceorigin.listener =
    /// name`, and their rows' `listener`, telling apart the listeners of one
    /// service.
    #[must_use]
    pub const fn listener(self, name: &'static str) -> Self {
        Self {
            listener: Some(name),
            ..self
        }
    }

    /// Wraps every route and the fallback of `router`.
    pub fn wrap<S: Clone + Send + Sync + 'static>(self, router: Router<S>) -> Router<S> {
        router.layer(axum::middleware::from_fn_with_state(self, Self::trace))
    }

    /// Runs the request in its span, then records its outcome.
    async fn trace(State(this): State<Self>, request: Request, next: Next) -> Response {
        let started = Instant::now();
        let method = METHODS
            .iter()
            .find(|known| *known == request.method())
            .map_or("_OTHER", Method::as_str);
        let route = request
            .extensions()
            .get::<MatchedPath>()
            .map(Route::matched);
        let span = tracing::info_span!(
            parent: None,
            "http.server",
            trace_id = Empty,
            otel.name = route.as_ref().map_or_else(|| method.to_owned(), |route| format!("{method} {}", route.as_str())),
            otel.kind = "server",
            otel.status_code = Empty,
        );
        let trace_id = Telemetry::trace_id(&span);
        if let Some(trace_id) = trace_id {
            span.record("trace_id", tracing::field::display(trace_id));
        }
        let mut labels = vec![KeyValue::new("http.request.method", method)];
        labels.extend(
            this.listener
                .map(|listener| KeyValue::new("sourceorigin.listener", listener)),
        );
        labels.extend(
            route
                .as_ref()
                .map(|route| KeyValue::new("http.route", route.as_str().to_owned())),
        );
        let mut response = next.run(request).instrument(span.clone()).await;
        let status = response.status().as_u16();
        labels.push(KeyValue::new(
            "http.response.status_code",
            i64::from(status),
        ));
        for label in &labels {
            span.set_attribute(label.key.clone(), label.value.clone());
        }
        if response.status().is_server_error() {
            span.record("otel.status_code", "ERROR");
            if let Some(value) = trace_id.and_then(|id| HeaderValue::try_from(id.to_string()).ok())
            {
                response.headers_mut().insert(Self::TRACE_ID_HEADER, value);
            }
        }
        let elapsed = started.elapsed();
        let duration_ms = elapsed.as_millis();
        let quiet = route
            .as_ref()
            .is_some_and(|route| this.quiet.contains(&route.as_str()));
        span.in_scope(|| {
            let target = route.as_ref().map_or(UNMATCHED, Route::as_str);
            macro_rules! access {
                ($level:ident) => {
                    tracing::$level!(
                        http.response.status_code = status,
                        sourceorigin.listener = this.listener,
                        duration_ms,
                        "{method} {target}"
                    )
                };
            }
            if quiet {
                access!(debug);
            } else {
                access!(info);
            }
        });
        if quiet {
            return response;
        }
        RequestServed {
            service: SERVICE.get().copied().unwrap_or("unknown"),
            instance: Instance::current(),
            listener: this.listener,
            method,
            route,
            status,
            duration_us: u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX),
            trace: trace_id,
        }
        .record();
        response
    }
}

#[cfg(test)]
mod tests;
