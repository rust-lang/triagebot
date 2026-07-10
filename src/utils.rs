use crate::handlers::Context;

use anyhow::Context as _;
use axum::http::HeaderValue;
use globset::GlobSet;
use hyper::{
    HeaderMap,
    header::{CACHE_CONTROL, CONTENT_TYPE},
};
use std::{borrow::Cow, path::Path};

/// Pluralize (add an 's' sufix) to `text` based on `count`.
pub fn pluralize(text: &str, count: usize) -> Cow<'_, str> {
    if count == 1 {
        text.into()
    } else {
        format!("{text}s").into()
    }
}

/// Can triagebot provide extended GitHub features (such as comments, logs, etc.)
/// for this repository for unauthorized users?
pub(crate) async fn is_known_and_public_repo(
    ctx: &Context,
    owner: &str,
    repo: &str,
) -> anyhow::Result<bool> {
    let repos = ctx
        .team
        .repos()
        .await
        .context("unable to retrieve team repos")?;

    // Verify that the request org is part of the Rust project
    let Some(repos) = repos.repos.get(owner) else {
        return Ok(false);
    };

    let repo = repos.iter().find(|r| r.name == repo);
    // Verify that the request repo is part of the Rust project
    let Some(repo) = repo else {
        return Ok(false);
    };

    // Only allow public repositories
    if repo.private {
        return Ok(false);
    }

    Ok(true)
}

pub(crate) async fn is_issue_under_rfcbot_fcp(
    issue_full_repo_name: &str,
    issue_number: u64,
) -> bool {
    match crate::rfcbot::get_all_fcps().await {
        Ok(fcps) => {
            if fcps.iter().any(|(_, fcp)| {
                u64::from(fcp.issue.number) == issue_number
                    && fcp.issue.repository == issue_full_repo_name
            }) {
                return true;
            }
        }
        Err(err) => {
            tracing::warn!("unable to fetch rfcbot active FCPs: {err:?}, skipping check");
        }
    }

    false
}

pub(crate) fn immutable_headers(content_type: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();

    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=15552000, immutable"),
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));

    headers
}

#[derive(Debug)]
pub(crate) struct ModifiedPathMatcher {
    prefixes: Vec<String>,
    globs: GlobSet,
}

impl ModifiedPathMatcher {
    /// Create a matcher against a set of prefixes (default) and globs (if the entry
    /// contains wildcards).
    pub fn new<'a, S>(entries: &[S]) -> Self
    where
        S: AsRef<str>,
    {
        let mut prefixes = Vec::new();
        let mut globs = GlobSet::builder();

        for entry in entries {
            let entry = entry.as_ref();
            if globset::escape(entry) == entry {
                prefixes.push(entry.to_owned());
                continue;
            }

            // Prepare the glob pattern from the entry.
            //
            // We first trim any excess `/` at the end of the pattern and then add an alternate
            // to match on the path as is or in any sub-directories.
            let pattern = entry.trim_end_matches('/');
            let pattern = format!("{pattern}{{,/*}}");

            // Create the glob pattern and log an error (should have already been reported to
            // the user).
            let glob = match globset::GlobBuilder::new(&pattern)
                .empty_alternates(true)
                .build()
            {
                Ok(pattern) => pattern,
                Err(err) => {
                    tracing::error!("invalid glob pattern for \"{entry}\": {err}");
                    continue;
                }
            };

            globs.add(glob);
        }

        let globs = match globs.build() {
            Ok(globs) => globs,
            Err(err) => {
                // Shouldn't fail since the globs are already validated, but `globset` returns
                // a `Result`.
                tracing::error!("unable to build glob pattern: {err}");
                GlobSet::empty()
            }
        };

        Self { prefixes, globs }
    }

    /// Create a matcher against a single prefix or glob.
    pub fn single(entry: &str) -> Self {
        Self::new(&[entry])
    }

    pub fn is_match(&self, path: impl AsRef<Path>) -> bool {
        let path = path.as_ref();

        if self.globs.is_match(path) {
            return true;
        }

        for pfx in &self.prefixes {
            if path.starts_with(pfx) {
                return true;
            }
        }

        false
    }

    /// Check for invalid globs or absolute paths.
    pub fn validate_entry(entry: &str) -> Result<(), PathMatcherError> {
        if let Err(e) = globset::Glob::new(entry) {
            return Err(PathMatcherError::Glob(e));
        }

        if entry.starts_with('/') {
            return Err(PathMatcherError::NonRelativePath);
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum PathMatcherError {
    Glob(globset::Error),
    NonRelativePath,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn modified_paths_matches(modified_paths: &[&Path], entry: &str) -> Vec<PathBuf> {
        modified_paths_matches_set(modified_paths, &[entry])
    }

    fn modified_paths_matches_set(modified_paths: &[&Path], entries: &[&str]) -> Vec<PathBuf> {
        let matcher = ModifiedPathMatcher::new(entries);
        modified_paths
            .iter()
            .filter(|p| matcher.is_match(p))
            .map(PathBuf::from)
            .collect()
    }

    #[test]
    fn entry_not_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("library/Cargo.lock"),
                    Path::new("library/Cargo.toml")
                ],
                "compiler/rustc_span/",
            ),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn entry_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("compiler/rustc_span/src/lib.rs"),
                    Path::new("compiler/rustc_span/src/symbol.rs"),
                ],
                "compiler/rustc_span/",
            ),
            vec![
                PathBuf::from("compiler/rustc_span/src/lib.rs"),
                PathBuf::from("compiler/rustc_span/src/symbol.rs"),
            ]
        );
    }

    #[test]
    fn entry_filename_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("compiler/rustc_span/src/lib.rs"),
                    Path::new("compiler/rustc_span/src/symbol.rs"),
                ],
                "compiler/rustc_span/src/lib.rs",
            ),
            vec![PathBuf::from("compiler/rustc_span/src/lib.rs")]
        );
    }

    #[test]
    fn entry_top_level_filename_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new(".git/submodules"),
                ],
                "Cargo.lock",
            ),
            vec![PathBuf::from("Cargo.lock")]
        );
    }

    #[test]
    fn entry_modified_glob_either() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                ],
                "library/{dec2flt,flt2dec}",
            ),
            vec![
                PathBuf::from("library/dec2flt/lib.rs"),
                PathBuf::from("library/flt2dec/lib.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_star() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                ],
                "library/dec2*",
            ),
            vec![PathBuf::from("library/dec2flt/lib.rs")]
        );
    }

    #[test]
    fn entry_modified_glob_star_middle() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("compiler/x86-64-none_eabi.rs"),
                    Path::new("compiler/armv7-none_eabi-something.rs"),
                    Path::new("compiler/armv7-none_eabi-something.txt"),
                    Path::new("compiler/none_eabi.rs"),
                ],
                "compiler/*none_eabi*.rs",
            ),
            vec![
                PathBuf::from("compiler/x86-64-none_eabi.rs"),
                PathBuf::from("compiler/armv7-none_eabi-something.rs"),
                PathBuf::from("compiler/none_eabi.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_double_star() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/.empty"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                ],
                "library/**",
            ),
            vec![
                PathBuf::from("library/.empty"),
                PathBuf::from("library/dec2flt/lib.rs"),
                PathBuf::from("library/flt2dec/lib.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_set() {
        assert_eq!(
            modified_paths_matches_set(
                &[
                    Path::new("Cargo.lock"),
                    Path::new("Cargo.toml"),
                    Path::new("library/.empty"),
                    Path::new("library/dec2flt/lib.rs"),
                    Path::new("library/flt2dec/lib.rs"),
                    Path::new("a/foo/glob1.rs"),
                    Path::new("b/glob1.rs"),
                    Path::new("glob1.rs"),
                    Path::new("pfx1"),
                    Path::new("pfx1/foo.rs"),
                    Path::new("pfx2.rs"),
                    Path::new("pfx2/foo.rs"),
                    Path::new("nomatch/pfx1"),
                    Path::new("nomatch/pfx2")
                ],
                &[
                    // Two globs, two prefixes
                    "library/**",
                    "*glob1*",
                    "pfx1",
                    "pfx2",
                ]
            ),
            vec![
                PathBuf::from("library/.empty"),
                PathBuf::from("library/dec2flt/lib.rs"),
                PathBuf::from("library/flt2dec/lib.rs"),
                PathBuf::from("a/foo/glob1.rs"),
                PathBuf::from("b/glob1.rs"),
                PathBuf::from("glob1.rs"),
                PathBuf::from("pfx1"),
                PathBuf::from("pfx1/foo.rs"),
                PathBuf::from("pfx2/foo.rs"),
            ]
        );
    }

    #[test]
    fn entry_modified_glob_empty_alternates() {
        assert_eq!(
            modified_paths_matches(
                &[Path::new("result.rs"), Path::new("result.rs.stdout")],
                "result.rs{,.stdout}",
            ),
            vec![
                PathBuf::from("result.rs"),
                PathBuf::from("result.rs.stdout"),
            ]
        );
    }

    #[test]
    fn entry_submodule_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
    }

    #[test]
    fn entry_submodule_and_normal_dir_modified() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo{,test}"
            ),
            vec![
                PathBuf::from("src/tools/cargo"),
                PathBuf::from("src/tools/cargotest")
            ]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo*"
            ),
            vec![
                PathBuf::from("src/tools/cargo"),
                PathBuf::from("src/tools/cargotest")
            ]
        );
    }

    #[test]
    fn entry_submodule_modified_with_trailing_slash() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo/"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/tools/cargo//"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
    }

    #[test]
    fn entry_submodule_modified_glob() {
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/*/cargo"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/*/cargo/"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
        assert_eq!(
            modified_paths_matches(
                &[
                    Path::new("src/tools/cargo"),
                    Path::new("src/tools/cargotest")
                ],
                "src/*/cargo//"
            ),
            vec![PathBuf::from("src/tools/cargo")]
        );
    }
}
