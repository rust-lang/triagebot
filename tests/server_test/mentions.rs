use super::run_test;

#[test]
fn default_mention() {
    // A new PR that touches a file in the [mentions] config with the default
    // message.
    run_test("mentions/default_mention");
}

#[test]
fn custom_message() {
    // A new PR that touches a file in the [mentions] config with a custom
    // message.
    run_test("mentions/custom_message");
}

#[test]
fn dont_mention_twice() {
    // When pushing modifications to the same files, don't mention again.
    //
    // However if a push comes in for a different file, make sure it mentions again.
    //
    // This starts with a new PR adding example2/README.md.
    // It then pushes an update to example2/README.md.
    // And then a second update to add example1/README.md.
    run_test("mentions/dont_mention_twice");
}
