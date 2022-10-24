// Function to match the scheduled job function with its corresponding handler.
// In case you want to add a new one, just add a new clause to the match with
// the job name and the corresponding function.

// The metadata is a serde_json::Value
// Please refer to https://docs.rs/serde_json/latest/serde_json/value/fn.from_value.html
// on how to interpret it as an instance of type T, implementing Serialize/Deserialize.

// For example, if we want to sends a Zulip message every Friday at 11:30am ET into #t-release
// with a @T-release meeting! content, we should create some Job like:
//
//    #[derive(Serialize, Deserialize)]
//    struct ZulipMetadata {
//      pub message: String
//    }
//
//    let metadata = serde_json::value::to_value(ZulipMetadata {
//      message: "@T-release meeting!".to_string()
//     }).unwrap();
//
//    Job {
//      name: "send_zulip_message",
//      scheduled_at: "2022-09-30T11:30:00+10:00",
//      metadata: metadata
//    }
//
// ... and add the corresponding "send_zulip_message" handler.

pub async fn handle_job(name: &String, metadata: &serde_json::Value) -> anyhow::Result<()> {
    match name {
        _ => default(&name, &metadata),
    }
}

fn default(name: &String, metadata: &serde_json::Value) -> anyhow::Result<()> {
    tracing::trace!(
        "handle_job fell into default case: (name={:?}, metadata={:?})",
        name,
        metadata
    );

    Ok(())
}
