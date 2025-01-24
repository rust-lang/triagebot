use crate::db;
use crate::db::make_client;
use crate::db::notifications::record_username;
use std::future::Future;
use tokio_postgres::Config;

/// Represents a connection to a Postgres database that can be
/// used in integration tests to test logic that interacts with
/// a database.
pub struct TestContext {
    client: tokio_postgres::Client,
    db_name: String,
    original_db_url: String,
    conn_handle: tokio::task::JoinHandle<()>,
}

impl TestContext {
    async fn new(db_url: &str) -> Self {
        let mut config: Config = db_url.parse().expect("Cannot parse connection string");

        // Create a new database that will be used for this specific test
        let client = make_client(&db_url)
            .await
            .expect("Cannot connect to database");
        let db_name = format!("db{}", uuid::Uuid::new_v4().to_string().replace("-", ""));
        client
            .execute(&format!("CREATE DATABASE {db_name}"), &[])
            .await
            .expect("Cannot create database");
        drop(client);

        // We need to connect to the database against, because Postgres doesn't allow
        // changing the active database mid-connection.
        config.dbname(&db_name);
        let (mut client, connection) = config
            .connect(tokio_postgres::NoTls)
            .await
            .expect("Cannot connect to the newly created database");
        let conn_handle = tokio::spawn(async move {
            connection.await.unwrap();
        });

        db::run_migrations(&mut client)
            .await
            .expect("Cannot run database migrations");
        Self {
            client,
            db_name,
            original_db_url: db_url.to_string(),
            conn_handle,
        }
    }

    pub fn db_client(&self) -> &tokio_postgres::Client {
        &self.client
    }

    pub async fn add_user(&self, name: &str, id: u64) {
        record_username(&self.client, id, name)
            .await
            .expect("Cannot create user");
    }

    async fn finish(self) {
        // Cleanup the test database
        // First, we need to stop using the database
        drop(self.client);
        self.conn_handle.await.unwrap();

        // Then we need to connect to the default database and drop our test DB
        let client = make_client(&self.original_db_url)
            .await
            .expect("Cannot connect to database");
        client
            .execute(&format!("DROP DATABASE {}", self.db_name), &[])
            .await
            .unwrap();
    }
}

pub async fn run_test<F, Fut>(f: F)
where
    F: FnOnce(TestContext) -> Fut,
    Fut: Future<Output = anyhow::Result<TestContext>>,
{
    if let Ok(db_url) = std::env::var("TEST_DB_URL") {
        let ctx = TestContext::new(&db_url).await;
        let ctx = f(ctx).await.expect("Test failed");
        ctx.finish().await;
    } else {
        eprintln!("Skipping test because TEST_DB_URL was not passed");
    }
}
