use crate::db::rustc_commits;
use crate::db::rustc_commits::get_missing_commits;
use crate::{
    github::{self, Event},
    handlers::Context,
};
use std::collections::VecDeque;
use std::convert::TryInto;
use tracing as log;
use crate::db::events::get_events;


pub async fn handle(ctx: &Context, event: &Event) -> anyhow::Result<()> {
    let db = ctx.db.get().await;
    let res = get_events(&db).await?;
    println!("result: {:#?}", res);
    Ok(())

}
