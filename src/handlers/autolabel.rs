use crate::{
    config::AutolabelConfig,
    github::{IssuesAction, IssuesEvent, Label},
    handlers::Context,
};
pub(super) struct AutolabelInput {
    labels: Vec<Label>,
}

pub(super) async fn parse_input(
    ctx: &Context,
    event: &IssuesEvent,
    config: Option<&AutolabelConfig>,
) -> Result<Option<AutolabelInput>, String> {
    if let Some(diff) = event
        .diff_between(&ctx.github)
        .await
        .map_err(|e| {
            log::error!("failed to fetch diff: {:?}", e);
        })
        .unwrap_or_default()
    {
        if let Some(config) = config {
            let files = extract_files_from_diff(&diff);
            let mut autolabels = Vec::new();
            for changed_file in files {
                if changed_file.is_empty() {
                    // TODO: when would this be true?
                    continue;
                }
                for (label, cfg) in config.labels.iter() {
                    if cfg
                        .trigger_files
                        .iter()
                        .any(|f| changed_file.starts_with(f))
                    {
                        autolabels.push(Label {
                            name: label.to_owned(),
                        });
                    }
                }
                if !autolabels.is_empty() {
                    return Ok(Some(AutolabelInput { labels: autolabels }));
                }
            }
        }
    }

    if event.action == IssuesAction::Labeled {
        if let Some(config) = config {
            let mut autolabels = Vec::new();
            let applied_label = &event.label.as_ref().expect("label").name;

            'outer: for (label, config) in config.get_by_trigger(applied_label) {
                let exclude_patterns: Vec<glob::Pattern> = config
                    .exclude_labels
                    .iter()
                    .filter_map(|label| match glob::Pattern::new(label) {
                        Ok(exclude_glob) => Some(exclude_glob),
                        Err(error) => {
                            log::error!("Invalid glob pattern: {}", error);
                            None
                        }
                    })
                    .collect();

                for label in event.issue.labels() {
                    for pat in &exclude_patterns {
                        if pat.matches(&label.name) {
                            // If we hit an excluded label, ignore this autolabel and check the next
                            continue 'outer;
                        }
                    }
                }

                // If we reach here, no excluded labels were found, so we should apply the autolabel.
                autolabels.push(Label {
                    name: label.to_owned(),
                });
            }
            if !autolabels.is_empty() {
                return Ok(Some(AutolabelInput { labels: autolabels }));
            }
        }
    }
    if event.action == IssuesAction::Closed {
        let labels = event.issue.labels();
        if let Some(x) = labels.iter().position(|x| x.name == "I-prioritize") {
            let mut labels_excluded = labels.to_vec();
            labels_excluded.remove(x);
            return Ok(Some(AutolabelInput {
                labels: labels_excluded,
            }));
        }
    }
    Ok(None)
}

fn extract_files_from_diff(diff: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in diff.lines() {
        // mostly copied from highfive
        if line.starts_with("diff --git ") {
            let parts = line[line.find(" b/").unwrap() + " b/".len()..].split("/");
            let path = parts.collect::<Vec<_>>().join("/");
            if !path.is_empty() {
                files.push(path);
            }
        }
    }
    files
}

pub(super) async fn handle_input(
    ctx: &Context,
    _config: &AutolabelConfig,
    event: &IssuesEvent,
    input: AutolabelInput,
) -> anyhow::Result<()> {
    let mut labels = event.issue.labels().to_owned();
    for label in input.labels {
        // Don't add the label if it's already there
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    event.issue.set_labels(&ctx.github, labels).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_one_file() {
        let input = r##"\
diff --git a/triagebot.toml b/triagebot.toml
index fb9cee43b2d..b484c25ea51 100644
--- a/triagebot.toml
+++ b/triagebot.toml
@@ -114,6 +114,15 @@ trigger_files = [
        "src/tools/rustdoc-themes",
    ]

+[autolabel."T-compiler"]
+trigger_files = [
+    # Source code
+    "compiler",
+
+    # Tests
+    "src/test/ui",
+]
+
    [notify-zulip."I-prioritize"]
    zulip_stream = 245100 # #t-compiler/wg-prioritization/alerts
    topic = "#{number} {title}"
         "##;
        assert_eq!(
            extract_files_from_diff(input),
            vec!["triagebot.toml".to_string()]
        );
    }

    #[test]
    fn extract_several_files() {
        let input = r##"\
diff --git a/library/stdarch b/library/stdarch
index b70ae88ef2a..cfba59fccd9 160000
--- a/library/stdarch
+++ b/library/stdarch
@@ -1 +1 @@
-Subproject commit b70ae88ef2a6c83acad0a1e83d5bd78f9655fd05
+Subproject commit cfba59fccd90b3b52a614120834320f764ab08d1
diff --git a/src/librustdoc/clean/types.rs b/src/librustdoc/clean/types.rs
index 1fe4aa9023e..f0330f1e424 100644
--- a/src/librustdoc/clean/types.rs
+++ b/src/librustdoc/clean/types.rs
@@ -2322,3 +2322,4 @@ impl SubstParam {
        if let Self::Lifetime(lt) = self { Some(lt) } else { None }
    }
}
+
diff --git a/src/librustdoc/core.rs b/src/librustdoc/core.rs
index c58310947d2..3b0854d4a9b 100644
--- a/src/librustdoc/core.rs
+++ b/src/librustdoc/core.rs
@@ -591,3 +591,4 @@ fn from(idx: u32) -> Self {
        ImplTraitParam::ParamIndex(idx)
    }
}
+
"##;
        assert_eq!(
            extract_files_from_diff(input),
            vec![
                "library/stdarch".to_string(),
                "src/librustdoc/clean/types.rs".to_string(),
                "src/librustdoc/core.rs".to_string(),
            ]
        )
    }
}
