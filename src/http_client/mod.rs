use anyhow::Result;
use async_trait::async_trait;
use reqwest::Url;

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct CompilerMeetings {
    items: Vec<CompilerMeeting>,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct Start {
    #[serde(rename(deserialize = "dateTime"))]
    date_time: String,
}

#[derive(serde::Deserialize, serde::Serialize, Debug)]
pub struct CompilerMeeting {
    summary: String,
    #[serde(rename(deserialize = "htmlLink"))]
    html_link: String,
    #[serde(rename(deserialize = "originalStartTime"))]
    original_start: Option<Start>,
    start: Option<Start>,
}

#[async_trait]
pub trait HttpClient {
    async fn get_meetings(
        start_date: chrono::DateTime<chrono::Local>,
        end_date: chrono::DateTime<chrono::Local>,
    ) -> Result<Vec<CompilerMeeting>>
    where
        Self: Sized;
}

#[async_trait]
impl HttpClient for CompilerMeeting {
    /// Retrieve all meetings from the Rust Compiler Team Calendar in a date range
    /// If a Google API auth token is not provided just return
    // Google calendar API documentation:
    // https://developers.google.com/calendar/api/v3/reference/events/list
    // The API token needs only one permission: https://www.googleapis.com/auth/calendar.events.readonly
    async fn get_meetings(
        start_date: chrono::DateTime<chrono::Local>,
        end_date: chrono::DateTime<chrono::Local>,
    ) -> Result<Vec<CompilerMeeting>> {
        let api_key = match std::env::var("GOOGLE_API_KEY") {
            Ok(v) => v,
            Err(_) => {
                return Ok(vec![]);
            }
        };
        let google_calendar_id = "6u5rrtce6lrtv07pfi3damgjus%40group.calendar.google.com";
        let time_min = format!("{}T00:00:00+00:00", start_date.format("%F"));
        let time_max = format!("{}T23:59:59+00:00", end_date.format("%F"));
        let url = Url::parse_with_params(
            &format!(
                "https://www.googleapis.com/calendar/v3/calendars/{}/events",
                google_calendar_id
            ),
            &[
                // see google docs for the meaning of these values
                ("key", api_key),
                ("timeMin", time_min),
                ("timeMax", time_max),
                ("singleEvents", "true".to_string()),
                ("orderBy", "startTime".to_string()),
            ],
        )?;
        let calendar = reqwest::get(url).await?.json::<CompilerMeetings>().await?;
        Ok(calendar.items)
    }
}
