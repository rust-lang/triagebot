use crate::db;
use crate::db::notifications::record_username;
use crate::db::{make_client, ClientPool, PooledClient};
use std::future::Future;
use tokio_postgres::config::Host;
use tokio_postgres::{Config, GenericClient};

/// Represents a connection to a Postgres database that can be
/// used in integration tests to test logic that interacts with
/// a database.
pub struct TestContext {
    pool: ClientPool,
    db_name: String,
    original_db_url: String,
}

impl TestContext {
    async fn new(db_url: &str) -> Self {
        let config: Config = db_url.parse().expect("Cannot parse connection string");

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
        // There does not seem to be a way to turn the config back into a connection
        // string, so construct it manually.
        let test_db_url = format!(
            "postgresql://{}:{}@{}/{}",
            config.get_user().unwrap(),
            String::from_utf8(config.get_password().unwrap().to_vec()).unwrap(),
            match &config.get_hosts()[0] {
                Host::Tcp(host) => host,
                Host::Unix(_) =>
                    panic!("Unix sockets in Postgres connection string are not supported"),
            },
            db_name
        );
        let pool = ClientPool::new(test_db_url);
        db::run_migrations(&mut *pool.get().await)
            .await
            .expect("Cannot run database migrations");
        Self {
            pool,
            db_name,
            original_db_url: db_url.to_string(),
        }
    }

    pub async fn db_client(&self) -> PooledClient {
        self.pool.get().await
    }

    pub async fn add_user(&self, name: &str, id: u64) {
        record_username(self.db_client().await.client(), id, name)
            .await
            .expect("Cannot create user");
    }

    async fn finish(self) {
        // Cleanup the test database
        // First, we need to stop using the database
        drop(self.pool);

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
