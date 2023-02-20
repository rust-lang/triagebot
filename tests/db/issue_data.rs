use super::run_test;
use serde::{Deserialize, Serialize};
use triagebot::db::issue_data::IssueData;

#[derive(Serialize, Deserialize, Default, Debug)]
struct MyData {
    f1: String,
    f2: u32,
}

#[test]
fn issue_data() {
    run_test(|mut connection| async move {
        let repo = "rust-lang/rust".to_string();
        let mut id: IssueData<MyData> =
            IssueData::load(&mut *connection, repo.clone(), 1234, "test")
                .await
                .unwrap();
        assert_eq!(id.data.f1, "");
        assert_eq!(id.data.f2, 0);
        id.data.f1 = "new data".to_string();
        id.data.f2 = 1;
        id.save().await.unwrap();
        let id: IssueData<MyData> = IssueData::load(&mut *connection, repo.clone(), 1234, "test")
            .await
            .unwrap();
        assert_eq!(id.data.f1, "new data");
        assert_eq!(id.data.f2, 1);
    });
}
