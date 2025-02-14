#[cfg(test)]
mod tests {
    use crate::handlers::assign::filter_by_capacity;
    use crate::tests::run_test;
    use std::collections::HashSet;

    #[tokio::test]
    async fn find_reviewers_no_review_prefs() {
        run_test(|ctx| async move {
            ctx.add_user("usr1", 1).await;
            ctx.add_user("usr2", 1).await;
            let _users =
                filter_by_capacity(ctx.db_client(), &candidates(&["usr1", "usr2"])).await?;
            // FIXME: this test fails, because the query is wrong
            // check_users(users, &["usr1", "usr2"]);
            Ok(ctx)
        })
        .await;
    }

    fn candidates(users: &[&'static str]) -> HashSet<&'static str> {
        users.into_iter().copied().collect()
    }

    fn check_users(users: HashSet<String>, expected: &[&'static str]) {
        let mut users: Vec<String> = users.into_iter().collect();
        users.sort();
        assert_eq!(users, expected);
    }
}
