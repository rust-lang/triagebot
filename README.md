# Triagebot

This is the triage and team assistance bot for the rust-lang organization.

Please see the [wiki] for our documentation, and feel free to contribute edits
if you find something helpful!

[wiki]: https://github.com/rust-lang/triagebot/wiki

## How triagebot works

Triagebot consists of a webserver with several endpoints.
The `/github-hook` and `/zulip-hook` endpoints receive webhook notifications from the respective services.
Triagebot can then respond to those notifications to perform various actions such as adjusting labels.

The Triagebot webserver also includes several other endpoints intended for users to access directly, such as https://triage.rust-lang.org/agenda.

Triagebot uses a Postgres database to retain some state.
In production, it uses [RDS](https://aws.amazon.com/rds/).

The server at https://triage.rust-lang.org/ runs on ECS and is configured via [Terraform](https://github.com/rust-lang/simpleinfra/blob/master/terraform/shared/services/triagebot/main.tf#L8).
Updates are automatically deployed when merged to master.

## Installation

To compile the Triagebot you need OpenSSL development library to be installed (e.g. for Ubuntu-like Linux distributions `sudo apt install libssl-dev`).

Run `cargo build` to compile the triagebot.

## Running triagebot

It is possible to run triagebot yourself, and test changes against your own repository.
Some developers may settle with testing in production as the risks tend to be low, but the more intrepid may find it easier to iterate separately.

The general overview of what you will need to do:

1. Install Postgres. Look online for any help with installing and setting up Postgres (particularly if you need to create a user and set up permissions).
2. Create a database: `createdb triagebot`
3. Provide a way for GitHub to access the Triagebot webserver.
   There are various ways to do this (such as placing it behind a proxy, or poking holes in your firewall).
   Or, you can use a service such as https://ngrok.com/ to access on your local dev machine via localhost.
   Installation is fairly simple, though requires setting up a (free) account.
   Run the command `ngrok http 8000` to forward to port 8000 on localhost.

   > Note: GitHub has a webhook forwarding service available in beta.
   > See [cli/gh-webhook](https://docs.github.com/en/developers/webhooks-and-events/webhooks/receiving-webhooks-with-the-github-cli) for more information.
   > This is super easy to use, and doesn't require manually configuring webhook settings.
   > The command to run looks something like:
   >
   > ```sh
   > gh webhook forward --repo=ehuss/triagebot-test --events=* \
   >     --url=http://127.0.0.1:8000/github-hook --secret somelongsekrit
   > ```
   >
   > Where the value in `--secret` is the secret value you place in `GITHUB_WEBHOOK_SECRET` described below, and `--repo` is the repo you want to test against.

4. Create a GitHub repo to run some tests on.
5. Configure the webhook in your GitHub repo.
   I recommend at least skimming the [GitHub webhook documentation](https://docs.github.com/en/developers/webhooks-and-events/webhooks/about-webhooks) if you are not familiar with webhooks. In short:

   1. Go to the settings page.
   2. Go to the webhook section.
   3. Click "Add webhook"
   4. Include the settings:

      - Payload URL: This is the URL to your Triagebot server, for example http://7e9ea9dc.ngrok.io/github-hook. This URL is displayed when you ran the `ngrok` command above.
      - Content type: application/json
      - Secret: Enter a shared secret (some longish random text)
      - Events: "Send me everything"
6. Configure the `.env` file:

   1. Copy `.env.sample` to `.env`
   2. `GITHUB_API_TOKEN`: This is a token needed for Triagebot to send requests to GitHub. Go to GitHub Settings > Developer Settings > Personal Access Token, and create a new token. The `repo` permission should be sufficient.
      If this is not set, Triagebot will also look in `~/.gitconfig` in the `github.oauth-token` setting.
   3. `DATABASE_URL`: This is the URL to the Postgres database. Something like `postgres://eric@localhost/triagebot` should work, replacing `eric` with your username.
   4. `GITHUB_WEBHOOK_SECRET`: Enter the secret you entered in the webhook above.
   5. `RUST_LOG`: Set this to `debug`.

7. Run `cargo run --bin triagebot`. This starts the http server listening on port 8000.
8. Add a `triagebot.toml` file to the main branch of your GitHub repo with whichever services you want to try out.
9. Try interacting with your repo, such as issuing `@rustbot` commands or interacting with PRs and issues (depending on which services you enabled in `triagebot.toml`). Watch the logs from the server to see what's going on.

## Tests

When possible, writing unittests is very helpful and one of the easiest ways to test.
For more advanced testing, there is an integration test called `testsuite` which provides an end-to-end service for testing triagebot.
There are two parts to it:

* [`github_client`](tests/github_client/mod.rs) — Tests specifically targeting `GithubClient`.
  This sets up an HTTP server that mimics api.github.com and verifies the client's behavior.
* [`server_test`](tests/server_test/mod.rs) — This tests the `triagebot` server itself and its behavior when it receives a webhook.
  This launches the `triagebot` server, sets up HTTP servers to intercept api.github.com requests, launches PostgreSQL in a sandbox, and then injects webhook events into the `triagebot` server and validates its response.

The real GitHub API responses are recorded in JSON files that the tests can later replay to verify the behavior of triagebot.
These recordings are enabled with the `TRIAGEBOT_TEST_RECORD` environment variable.
See the documentation in `github_client` and `server_test` for the steps for setting up recording to write a test.

## License

Triagebot is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
