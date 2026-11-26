//! `/device`: where a user approves the sign-in of a device that shows them a
//! code (RFC 8628), from a browser signed in to their account.
//!
//! Its `GET` asks for the code, filled in when the device's link carried it. Its
//! `POST` looks the code up and shows what the device asks, then takes the
//! decision that screen posts. A browser that is not signed in signs in first,
//! and comes back. Nothing is approved without the screen naming the application:
//! a code in a link only fills the field in.

use crate::{
    oidc::{DeviceDecision, DeviceLookup, UserCode},
    routes,
    security::{DEVICE_LIFETIME_DAYS, SESSION_LIFETIME_DAYS},
    web::{Failure, WebApplication},
};
use sourceorigin_common::Handle;
use sourceorigin_ui::{
    DeviceCodeDecision, DeviceCodeForm, DeviceCodeQuery, LoginQuery, Query, Title,
};
use topcoat::{
    context::{Cx, app_context},
    router::{
        Body, StatusCode,
        error::see_other,
        request::headers,
        response::{IntoResponse, Response},
    },
    view::view,
};

/// What the page says of a code no device authorization waits under.
const UNKNOWN_CODE: &str = "No device is waiting for this code. Check it against your device, or start again there: a code works once, for 10 minutes.";
/// What the page says once the browser tried too many codes.
const TOO_MANY_CODES: &str = "Too many codes tried. Wait a minute, then try again.";

impl WebApplication {
    /// `GET /device`: asks for the code a device shows, starting with the one its
    /// link carried, once the browser is signed in.
    pub(in crate::web) async fn device(cx: &Cx) -> Result<Response, Failure> {
        let web = app_context::<Self>(cx);
        let query: DeviceCodeQuery = Self::query(cx)?;
        if web.owner(cx).await?.is_none() {
            return Self::sign_in_for_device(cx, &query);
        }
        let code = query.user_code.unwrap_or_default();
        web.device_code_page(cx, StatusCode::OK, &code, None).await
    }

    /// `POST /device`: shows what the device under the typed code asks, or, from
    /// that screen, records the decision and tells the user to go back to the
    /// device.
    pub(in crate::web) async fn decide_device(cx: &Cx, body: Body) -> Result<Response, Failure> {
        let web = app_context::<Self>(cx);
        let context = Self::context(cx)?;
        let form: DeviceCodeForm = Self::form(cx, body).await?;
        if web.owner(cx).await?.is_none() {
            let query = DeviceCodeQuery {
                user_code: Some(form.user_code),
            };
            return Self::sign_in_for_device(cx, &query);
        }
        let Some(code) = UserCode::parse(&form.user_code) else {
            return web
                .device_code_page(
                    cx,
                    StatusCode::BAD_REQUEST,
                    &form.user_code,
                    Some(UNKNOWN_CODE),
                )
                .await;
        };
        let found = match form.decision {
            None => web
                .oidc
                .device_request(context, headers(cx), code)
                .await?
                .map(Found::Request),
            Some(decision) => {
                let decided = match decision {
                    DeviceCodeDecision::Approve => DeviceDecision::Approve,
                    DeviceCodeDecision::Deny => DeviceDecision::Deny,
                };
                web.oidc
                    .decide_device(context, headers(cx), code, decided)
                    .await?
                    .map(|client| Found::Decided(client, decision))
            }
        };
        let (status, alert) = match found {
            DeviceLookup::Found(found) => return web.device_page(cx, code, found).await,
            DeviceLookup::Unknown => (StatusCode::NOT_FOUND, UNKNOWN_CODE),
            DeviceLookup::Throttled => (StatusCode::TOO_MANY_REQUESTS, TOO_MANY_CODES),
        };
        web.device_code_page(cx, status, &form.user_code, Some(alert))
            .await
    }

    /// Sends a browser that is not signed in to sign in, then back to the device
    /// page with the code of `query`.
    fn sign_in_for_device(cx: &Cx, query: &DeviceCodeQuery) -> Result<Response, Failure> {
        let login = LoginQuery {
            return_to: Some(query.on(routes::page::DEVICE)?),
            ..LoginQuery::default()
        }
        .on(routes::page::LOGIN)?;
        Ok(see_other(login).into_response(cx)?)
    }

    /// The form asking for a code, starting with `code`, at `status`, saying
    /// `alert` when the last code was not accepted.
    async fn device_code_page(
        &self,
        cx: &Cx,
        status: StatusCode,
        code: &str,
        alert: Option<&str>,
    ) -> Result<Response, Failure> {
        let page = view! { cx =>
            sourceorigin_ui::device_code(action: routes::page::DEVICE, code: code, alert: alert)
        };
        Ok(self
            .render(cx, status, Title::DeviceCode.as_str(), &[], page)
            .await?)
    }

    /// The screen after a code was `found`: what its device asks, for a decision,
    /// or what the decision did.
    async fn device_page(
        &self,
        cx: &Cx,
        code: UserCode,
        found: Found,
    ) -> Result<Response, Failure> {
        let title = Title::DeviceCode.as_str();
        match found {
            Found::Request(request) => {
                let here = DeviceCodeQuery {
                    user_code: Some(code.to_string()),
                }
                .on(routes::page::DEVICE)?;
                let sign_in = LoginQuery {
                    return_to: Some(here),
                    ..LoginQuery::default()
                }
                .on(routes::page::LOGIN)?;
                let code = code.to_string();
                let page = view! { cx =>
                    sourceorigin_ui::device_approval(
                        client: &request.client,
                        registered_by: request.registered_by.as_ref().map(Handle::as_str),
                        account: request.account.as_str(),
                        scopes: &request.scopes,
                        resource: request.resource.as_ref(),
                        code: &code,
                        started: request.started_at,
                        days: SESSION_LIFETIME_DAYS,
                        longest_days: DEVICE_LIFETIME_DAYS,
                        action: routes::page::DEVICE,
                        sign_in: &sign_in,
                    )
                };
                Ok(self.render(cx, StatusCode::OK, title, &[], page).await?)
            }
            Found::Decided(client, decision) => {
                let page = view! { cx =>
                    sourceorigin_ui::device_decided(client: &client, decision: decision)
                };
                Ok(self.render(cx, StatusCode::OK, title, &[], page).await?)
            }
        }
    }
}

/// What the device page shows once a code was found.
enum Found {
    /// What the device asks, for the user to decide on.
    Request(crate::oidc::DeviceRequest),
    /// The decision the user made about this application's device.
    Decided(String, DeviceCodeDecision),
}
