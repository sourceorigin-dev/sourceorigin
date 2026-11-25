//! Who administers Origin Auth: the list, granting rights and revoking them.

use super::{AdminApplication as App, ReasonForm, ReasonPage};
use crate::{AdminError, Reason};
use serde::Deserialize;
use sourceorigin_common::UserId;
use sourceorigin_ui::{Action, AdministratorRow, Change};
use topcoat::{
    Result,
    context::Cx,
    router::{StatusCode, content::Form, href, path_param, response::Response, route},
    view::view,
};

path_param!(pub administrator: UserId, error = not_found);

/// `/administrators`: who administers Origin Auth.
#[route(GET "/administrators")]
pub(super) async fn list(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let administrators = App::of(cx)
        .service
        .list_administrators(&admin.operator)
        .await?;
    let inline = admin.operator.is_confirmed(super::FORM_TIME);
    let rows: Vec<_> = administrators
        .iter()
        .map(|administrator| AdministratorRow {
            administrator,
            account: App::account(cx, administrator.user_id),
            picture: App::picture(cx, administrator.picture),
            revoke: if administrator.user_id == admin.user_id {
                Err("You cannot change your own rights.")
            } else {
                Ok(Action {
                    href: href!(revoke, Administrator(administrator.user_id)).resolve(cx),
                    inline,
                })
            },
        })
        .collect();
    let (granting, rows) = (&href!(grant).resolve(cx), &rows);
    let page = view! { cx =>
        sourceorigin_ui::administrator_list(rows: rows, grant: granting.as_str())
    };
    App::render(cx, StatusCode::OK, "Administrators", page).await
}

/// What [`grant`] takes: by `GET`, the search for the account, from its query;
/// by `POST`, the account to grant and why, from its form. One type, since the
/// extractor reads a `GET`'s query as its form.
///
/// Not an [`Administered`](super::Administered): flattened fields reach their
/// type as text, so `page=2`, the pager's own link, and an empty `user_id` would
/// fail to decode. Its own `reason` defaults the same way.
#[derive(Clone, Deserialize)]
pub(super) struct GrantForm {
    /// The account to grant rights, which the button posts.
    user_id: Option<UserId>,
    /// What to search for.
    #[serde(default)]
    q: String,
    /// The page wanted, counting from 1.
    page: Option<u32>,
    /// Why, which the button's dialog asks for.
    #[serde(default)]
    reason: String,
}

/// `/administrators/grant`: finds the account, by handle, name, email or ID, then
/// makes it an administrator.
#[route([GET, POST] "/administrators/grant")]
pub(super) async fn grant(cx: &Cx, form: Option<Form<GrantForm>>) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let back = href!(list).resolve(cx);
    let (q, page) = form
        .as_ref()
        .map_or_default(|Form(form)| (form.q.clone(), form.page));
    let refusal = App::submit(cx, form, back, |input: GrantForm| async move {
        let user_id = input.user_id.ok_or(AdminError::UserNotFound)?;
        let reason = Reason::parse(input.reason)?;
        service
            .grant_administrator(&admin.operator, user_id, reason)
            .await
    })
    .await?;
    let action = href!(grant).resolve(cx);
    let (users, administrators, pager) = App::found_users(cx, admin, &q, page, &action).await?;
    let rows = App::user_rows(cx, &users, &administrators);
    let (q, rows, pager, refusal) = (&q, &rows, &pager, refusal.as_deref());
    let page = view! { cx =>
        sourceorigin_ui::grant_search(
            action: action.as_str(),
            q: q.as_str(),
            rows: rows,
            pages: pager,
            refusal: refusal,
        )
    };
    App::render(cx, StatusCode::OK, "Grant rights", page).await
}

/// `/administrators/{administrator}/revoke`: removes another administrator's
/// rights.
#[route([GET, POST] "/administrators/{administrator}/revoke")]
pub(super) async fn revoke(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let id = *path_param::<Administrator>(cx)?;
    let back = href!(list).resolve(cx);
    let page = ReasonPage {
        crumb: "Revoke rights",
        ..ReasonPage::new(Change::RevokeAdministrator, &back)
    };
    App::reason_change(cx, form, page, back.clone(), async |reason| {
        service
            .revoke_administrator(&admin.operator, id, reason)
            .await
    })
    .await
}
