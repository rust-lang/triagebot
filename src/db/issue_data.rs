//! The `issue_data` table provides a way to track extra metadata about an
//! issue/PR.
//!
//! Each issue has a unique "key" where you can store data under. Typically
//! that key should be the name of the handler. The data can be anything that
//! can be serialized to JSON.
//!
//! Note that this uses crude locking, so try to keep the duration between
//! loading and saving to a minimum.

use crate::db;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub struct IssueData<'db, T>
where
    T: for<'a> Deserialize<'a> + Serialize + Default + std::fmt::Debug + Sync,
{
    transaction: Box<dyn db::Transaction + 'db>,
    repo: String,
    issue_number: i32,
    key: String,
    pub data: T,
}

impl<'db, T> IssueData<'db, T>
where
    T: for<'a> Deserialize<'a> + Serialize + Default + std::fmt::Debug + Sync,
{
    pub async fn load(
        connection: &'db mut dyn db::Connection,
        repo: String,
        issue_number: i32,
        key: &str,
    ) -> Result<IssueData<'db, T>> {
        let (transaction, raw) = connection
            .lock_and_load_issue_data(&repo, issue_number, key)
            .await?;
        let data = match raw {
            Some(raw) => T::deserialize(raw)?,
            None => T::default(),
        };
        Ok(IssueData {
            transaction,
            repo,
            issue_number,
            key: key.to_string(),
            data,
        })
    }

    pub async fn save(mut self) -> Result<()> {
        let raw_data = serde_json::to_value(self.data)?;
        self.transaction
            .conn()
            .save_issue_data(&self.repo, self.issue_number, &self.key, &raw_data)
            .await?;
        self.transaction.commit().await?;
        Ok(())
    }
}
