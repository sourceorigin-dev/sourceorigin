//! Signup invitations, whatever the signup setting: mailed to an address, or
//! links to pass on.

use super::{AdminApplication as App, ReasonForm, ReasonPage};
use crate::{AdminError, InviteExpiry, Reason};
use chrono::Utc;
use serde::Deserialize;
use sourceorigin_common::{
    TokenId,
    admin::{CreatedInvitation, InvitationState},
    email::NormalizedEmail,
};
use sourceorigin_ui::{Action, Change, InvitationRow, Target};
use topcoat::{
    Result,
    context::Cx,
    router::{StatusCode, content::Form, href, path_param, response::Response, route},
    view::view,
};

path_param!(pub invitation: TokenId, error = not_found);

/// The form creating an invitation.
#[derive(Clone, Deserialize)]
struct CreateInvitation {
    /// The address to mail the link to, which the account created with it starts
    /// with, verified. Without one, the link is shown once to pass on.
    #[serde(default)]
    email: Option<String>,
    /// How long the link stays valid: 1 hour to 365 days, a number followed by `m`,
    /// `h` or `d`, e.g. `24h`.
    /// Seven days when absent.
    #[serde(default)]
    expires_in: Option<String>,
    /// Why: 1 to 500 characters, without control characters.
    #[serde(default)]
    reason: String,
}

/// `/invitations`: signup invitations, newest first, with the changes each
/// open one takes: a mailed one is resent only while email is on.
#[route(GET "/invitations")]
pub(super) async fn list(cx: &Cx) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let app = App::of(cx);
    let invitations = app.service.list_invitations(&admin.operator).await?;
    let (inline, now) = (admin.operator.is_confirmed(super::FORM_TIME), Utc::now());
    let rows: Vec<_> = invitations
        .iter()
        .map(|invitation| {
            let state = invitation.state(now);
            let action = |href: String| Action { href, inline };
            InvitationRow {
                invitation,
                resend: (state == InvitationState::Open
                    && invitation.email.is_some()
                    && app.service.mails())
                .then(|| action(href!(resend, Invitation(invitation.id)).resolve(cx))),
                revoke: matches!(state, InvitationState::Open | InvitationState::Expired)
                    .then(|| action(href!(revoke, Invitation(invitation.id)).resolve(cx))),
            }
        })
        .collect();
    let (creating, rows, required) = (&href!(create).resolve(cx), &rows, app.invite_required);
    let page = view! { cx =>
        sourceorigin_ui::invitation_list(
            rows: rows,
            create: creating.as_str(),
            now: now,
            required: required,
        )
    };
    App::render(cx, StatusCode::OK, "Invitations", page).await
}

/// `/invitations/new`: creates an invitation, mailed to the address given or
/// shown to pass on, then comes back here, which shows its link once, or where
/// it was mailed. The form asks for an address only while email is on.
#[route([GET, POST] "/invitations/new")]
pub(super) async fn create(cx: &Cx, form: Option<Form<CreateInvitation>>) -> Result<Response> {
    let admin = App::admin(cx).await?;
    let revealed: Option<CreatedInvitation> = App::revealed(cx, admin);
    if revealed.is_none() {
        App::confirm(cx, admin)?;
    }
    let app = App::of(cx);
    let back = href!(list).resolve(cx);
    let created = App::change(cx, form, |input: CreateInvitation| async move {
        let typed = |value: Option<String>| value.filter(|value| !value.trim().is_empty());
        let expires_in = match typed(input.expires_in) {
            Some(expires_in) => expires_in
                .trim()
                .parse::<InviteExpiry>()
                .map_err(AdminError::from)?,
            None => InviteExpiry::default(),
        };
        let email = typed(input.email)
            .map(|email| NormalizedEmail::parse(email.trim()))
            .transpose()
            .map_err(AdminError::from)?;
        let reason = Reason::parse(input.reason)?;
        app.service
            .create_invitation(
                &admin.operator,
                &app.invitation_links,
                expires_in,
                email,
                reason,
            )
            .await
    })
    .await?;
    let back = &back;
    let (created, refused) = match created {
        Some(Ok(created)) => return App::reveal(cx, admin, href!(create).resolve(cx), created),
        Some(Err(refused)) => (None, Some(refused)),
        None => (revealed, None),
    };
    let (email, expires_in, reason) = refused.as_ref().map_or((None, None, None), |refused| {
        (
            refused.form.email.as_deref(),
            refused.form.expires_in.as_deref(),
            Some(refused.form.reason.as_str()),
        )
    });
    let (created, refused) = (&created, refused.as_deref());
    let mails = app.service.mails();
    let page = view! { cx =>
        match created {
            Some(created) => {
                sourceorigin_ui::invitation_created(
                    sent: &created.sent,
                    expires_at: created.expires_at,
                    back: back.as_str(),
                )
            },
            None => {
                sourceorigin_ui::dashboard_change(
                    change: Change::Invite,
                    back: back.as_str(),
                    refusal: refused,
                    reason: reason,
                    if mails {
                        <label>
                            "Email address"
                            <input name="email" type="email" autocomplete="off" value=(email)>
                            <small>"Origin Auth mails the link here. Leave it empty to get a link to pass on yourself."</small>
                        </label>
                    }
                    <label>
                        "Valid for"
                        <input name="expires_in" placeholder="7d" pattern="[0-9]+[hd]" value=(expires_in)>
                        <small>"Hours or days, e.g. 24h or 7d; seven days when empty."</small>
                    </label>
                )
            },
        }
    };
    App::render(cx, StatusCode::OK, "Invite someone", page).await
}

/// `/invitations/{invitation}/resend`: mails an open invitation's link again.
#[route([GET, POST] "/invitations/{invitation}/resend")]
pub(super) async fn resend(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let id = *path_param::<Invitation>(cx)?;
    App::invitation_change(cx, form, Change::ResendInvitation, id, async |reason| {
        service.resend_invitation(&admin.operator, id, reason).await
    })
    .await
}

/// `/invitations/{invitation}/revoke`: revokes an unredeemed invitation.
#[route([GET, POST] "/invitations/{invitation}/revoke")]
pub(super) async fn revoke(cx: &Cx, form: Option<Form<ReasonForm>>) -> Result<Response> {
    let admin = App::confirmed(cx).await?;
    let service = &App::of(cx).service;
    let id = *path_param::<Invitation>(cx)?;
    App::invitation_change(cx, form, Change::RevokeInvitation, id, async |reason| {
        service.revoke_invitation(&admin.operator, id, reason).await
    })
    .await
}

impl App {
    /// The page of `change` to the invitation `id`, made with `call`, which goes
    /// back to the invitations once made.
    async fn invitation_change<F, Fut>(
        cx: &Cx,
        form: Option<Form<ReasonForm>>,
        change: Change,
        id: TokenId,
        call: F,
    ) -> Result<Response>
    where
        F: FnOnce(Reason) -> Fut,
        Fut: Future<Output = Result<(), AdminError>>,
    {
        let back = href!(list).resolve(cx);
        let shown = id.to_string();
        let page = ReasonPage {
            target: Some(Target {
                name: "Invitation",
                id: Some(&shown),
            }),
            ..ReasonPage::new(change, &back)
        };
        Self::reason_change(cx, form, page, back.clone(), call).await
    }
}
