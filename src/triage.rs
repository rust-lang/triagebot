use crate::handlers::Context;
use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse},
};
use chrono::{Duration, Utc};
use hyper::StatusCode;
use serde::Serialize;
use serde_json::value::{Value, to_value};
use std::sync::Arc;
use url::Url;

const YELLOW_DAYS: i64 = 7;
const RED_DAYS: i64 = 14;

pub async fn index() -> Html<&'static str> {
    Html(include_str!("../templates/triage/index.html"))
}

pub async fn pulls(
    Path((owner, repo)): Path<(String, String)>,
    State(ctx): State<Arc<Context>>,
) -> impl IntoResponse {
    let octocrab = &ctx.octocrab;
    let res = octocrab
        .pulls(&owner, &repo)
        .list()
        .sort(octocrab::params::pulls::Sort::Updated)
        .direction(octocrab::params::Direction::Ascending)
        .per_page(100)
        .send()
        .await;
    let Ok(mut page) = res else {
        return (
            StatusCode::NOT_FOUND,
            Html("The repository is not found.".to_string()),
        );
    };
    let mut base_pulls = page.take_items();
    let mut next_page = page.next;
    while let Some(mut page) = octocrab
        .get_page::<octocrab::models::pulls::PullRequest>(&next_page)
        .await
        .unwrap()
    {
        base_pulls.extend(page.take_items());
        next_page = page.next;
    }

    let mut pulls: Vec<Value> = Vec::new();
    for base_pull in base_pulls {
        let assignee = base_pull.assignee.map_or(String::new(), |v| v.login);
        let updated_at = base_pull
            .updated_at
            .map_or(String::new(), |v| v.format("%Y-%m-%d").to_string());

        let yellow_line = Utc::now() - Duration::days(YELLOW_DAYS);
        let red_line = Utc::now() - Duration::days(RED_DAYS);
        let need_triage = match base_pull.updated_at {
            Some(updated_at) if updated_at <= red_line => "red".to_string(),
            Some(updated_at) if updated_at <= yellow_line => "yellow".to_string(),
            _ => "green".to_string(),
        };
        let days_from_last_updated_at = if let Some(updated_at) = base_pull.updated_at {
            (Utc::now() - updated_at).num_days()
        } else {
            (Utc::now() - base_pull.created_at.unwrap()).num_days()
        };

        let labels = base_pull.labels.map_or(String::new(), |labels| {
            labels
                .iter()
                .map(|label| label.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        });
        let wait_for_author = labels.contains("S-waiting-on-author");
        let wait_for_review = labels.contains("S-waiting-on-review");
        let html_url = base_pull.html_url.unwrap();
        let number = base_pull.number;
        let title = base_pull.title.unwrap();
        let author = base_pull.user.unwrap().login;

        let pull = PullRequest {
            html_url,
            number,
            title,
            assignee,
            updated_at,
            need_triage,
            labels,
            author,
            wait_for_author,
            wait_for_review,
            days_from_last_updated_at,
        };
        pulls.push(to_value(pull).unwrap());
    }

    let mut context = tera::Context::new();
    context.insert("pulls", &pulls);
    context.insert("owner", &owner);
    context.insert("repo", &repo);

    let tera = tera::Tera::new("templates/triage/**/*").unwrap();
    let body = tera.render("pulls.html", &context).unwrap();

    (StatusCode::OK, Html(body))
}

#[derive(Serialize)]
struct PullRequest {
    pub html_url: Url,
    pub number: u64,
    pub title: String,
    pub assignee: String,
    pub updated_at: String,
    pub need_triage: String,
    pub labels: String,
    pub author: String,
    pub wait_for_author: bool,
    pub wait_for_review: bool,
    pub days_from_last_updated_at: i64,
}
