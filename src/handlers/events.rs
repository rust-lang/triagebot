// Function to match the scheduled event function with its corresponding handler.
// In case you want to add a new one, just add a new clause to the match with 
// the event name and the corresponding function.

// The metadata is a serde_json::Value, please visit: https://docs.rs/serde_json/latest/serde_json/value/enum.Value.html
// to refer on how to get values from there.
// Example of accessing an integer id in the metadata:
//    event_metadata["id"].as_i64().unwrap();

pub async fn handle_event(event_name: &String, event_metadata: &serde_json::Value) -> anyhow::Result<()> {
    match event_name {
      _ => default(&event_name, &event_metadata)
    }
}

fn default(event_name: &String, event_metadata: &serde_json::Value) -> anyhow::Result<()> {
  println!("handle_event fall in default cause: (name={:?}, metadata={:?})", event_name, event_metadata);
  tracing::trace!("handle_event fall in default cause: (name={:?}, metadata={:?})", event_name, event_metadata);

  Ok(())
}
