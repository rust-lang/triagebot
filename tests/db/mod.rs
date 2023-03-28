//! Tests for the database API.
//!
//! These tests help verify the database interaction. The [`run_test`]
//! function helps set up the database and gives your callback a connection to
//! interact with. The general form of a test is:
//!
//! ```rust
//! #[test]
//! fn example() {
//!     run_test(|mut connection| async move {
//!         // Call methods on `connection` and verify its behavior.
//!     });
//! }
//! ```
//!
//! The `run_test` function will run your test on both SQLite and Postgres (if
//! it is installed).

use futures::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use triagebot::db::{Connection, Pool};

mod issue_data;
mod jobs;
mod notification;
mod rustc_commits;

struct PgContext {
    db_dir: PathBuf,
    pool: Pool,
}

impl PgContext {
    fn new(db_dir: PathBuf) -> PgContext {
        let database_url = postgres_database_url(&db_dir);
        let pool = Pool::open(&database_url);
        PgContext { db_dir, pool }
    }
}

impl Drop for PgContext {
    fn drop(&mut self) {
        stop_postgres(&self.db_dir);
    }
}

struct SqliteContext {
    pool: Pool,
}

impl SqliteContext {
    fn new() -> SqliteContext {
        let db_path = super::test_dir().join("triagebot.sqlite3");
        let pool = Pool::open(db_path.to_str().unwrap());
        SqliteContext { pool }
    }
}

fn run_test<F, Fut>(f: F)
where
    F: Fn(Box<dyn Connection>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send,
{
    // Only run postgres if postgres can be found or on CI.
    if let Some(db_dir) = setup_postgres() {
        eprintln!("testing Postgres");
        let ctx = PgContext::new(db_dir);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async { f(ctx.pool.connection().await).await });
    } else if std::env::var_os("CI").is_some() {
        panic!("postgres must be installed in CI");
    }

    eprintln!("\n\ntesting Sqlite");
    let ctx = SqliteContext::new();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { f(ctx.pool.connection().await).await });
}

pub fn postgres_database_url(db_dir: &PathBuf) -> String {
    format!(
        "postgres:///triagebot?user=triagebot&host={}",
        db_dir.display()
    )
}

pub fn setup_postgres() -> Option<PathBuf> {
    let pg_dir = find_postgres()?;
    // Set up a directory where this test can store all its stuff.
    let test_dir = super::test_dir();
    let db_dir = test_dir.join("db");

    std::fs::create_dir(&db_dir).unwrap();
    let db_dir_str = db_dir.to_str().unwrap();
    run_command(
        &pg_dir.join("initdb"),
        &["--auth=trust", "--username=triagebot", "-D", db_dir_str],
        &db_dir,
    );
    run_command(
        &pg_dir.join("pg_ctl"),
        &[
            // -h '' tells it to not listen on TCP
            // -k tells it where to place the unix-domain socket
            "-o",
            &format!("-h '' -k {db_dir_str}"),
            // -D is the data dir where everything is stored
            "-D",
            db_dir_str,
            // -l enables logging to a file instead of stdout
            "-l",
            db_dir.join("postgres.log").to_str().unwrap(),
            "start",
        ],
        &db_dir,
    );
    run_command(
        &pg_dir.join("createdb"),
        &["--user", "triagebot", "-h", db_dir_str, "triagebot"],
        &db_dir,
    );
    Some(db_dir)
}

pub fn stop_postgres(db_dir: &Path) {
    // Shut down postgres.
    let pg_dir = find_postgres().unwrap();
    match Command::new(pg_dir.join("pg_ctl"))
        .args(&["-D", db_dir.to_str().unwrap(), "stop"])
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                eprintln!(
                    "failed to stop postgres:\n\
                    ---stdout\n\
                    {}\n\
                    ---stderr\n\
                    {}\n\
                    ",
                    std::str::from_utf8(&output.stdout).unwrap(),
                    std::str::from_utf8(&output.stderr).unwrap()
                );
            }
        }
        Err(e) => eprintln!("could not run pg_ctl to stop: {e}"),
    }
}

/// Finds the root for PostgreSQL commands.
///
/// For various reasons, some Linux distros hide some postgres commands and
/// don't put them on PATH, making them difficult to access.
fn find_postgres() -> Option<PathBuf> {
    // Check if on PATH first.
    if let Ok(o) = Command::new("initdb").arg("-V").output() {
        if o.status.success() {
            return Some(PathBuf::new());
        }
    }
    if let Ok(dirs) = std::fs::read_dir("/usr/lib/postgresql") {
        let mut versions: Vec<_> = dirs
            .filter_map(|entry| {
                let entry = entry.unwrap();
                // Versions are generally of the form 9.3 or 14, but this
                // might be broken if other forms are used.
                if let Ok(n) = entry.file_name().to_str().unwrap().parse::<f32>() {
                    Some((n, entry.path()))
                } else {
                    None
                }
            })
            .collect();
        if !versions.is_empty() {
            versions.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            return Some(versions.last().unwrap().1.join("bin"));
        }
    }
    None
}

fn run_command(command: &Path, args: &[&str], cwd: &Path) {
    eprintln!("running {command:?}: {args:?}");
    let output = Command::new(command)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("`{command:?}` failed to run: {e}"));
    if !output.status.success() {
        panic!(
            "{command:?} failed:\n\
            ---stdout\n\
            {}\n\
            ---stderr\n\
            {}\n\
            ",
            std::str::from_utf8(&output.stdout).unwrap(),
            std::str::from_utf8(&output.stderr).unwrap()
        );
    }
}
