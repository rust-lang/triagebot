# Triagebot

This is the triage and team assistance bot for the rust-lang organization.

Please see the [forge] for our documentation, and feel free to contribute edits
if you find something helpful!

[forge]: https://forge.rust-lang.org/triagebot/index.html

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

## Quick start

For local development/debugging for the log pages, do the following steps:

 1. Run `cp .env.sample .env`
 2. Change value of `SKIP_DB_MIGRATIONS` to `1`.
 3. Run `cargo run --bin triagebot`
 4. Go to this URL: <http://localhost:8000/gha-logs/rust-lang/rust/46814678314>

## Running triagebot

It is possible to run triagebot yourself, and test changes against your own repository.
Some developers may settle with testing in production as the risks tend to be low, but the more intrepid may find it easier to iterate separately.

The general overview of what you will need to do:

1. Create a repo on GitHub to run tests on.
2. [Configure a database](#configure-a-database)
3. [Configure webhook forwarding](#configure-webhook-forwarding)
4. Configure the `.env` file:

    1. Copy `.env.sample` to `.env`
    2. `GITHUB_TOKEN`: This is a token needed for Triagebot to send requests to GitHub. Go to GitHub Settings > Developer Settings > Personal Access Token, and create a new token. The `repo` permission should be sufficient.
       If this is not set, Triagebot will also look in `~/.gitconfig` in the `github.oauth-token` setting.
    3. `DATABASE_URL`: This is the URL to the database. See [Configuring a database](#configuring-a-database).
    4. `GITHUB_WEBHOOK_SECRET`: Enter the secret you entered in the webhook above.
    5. `RUST_LOG`: Set this to `debug`.

5. Run `cargo run --bin triagebot`. This starts the http server listening for webhooks on port 8000.
6. Add a `triagebot.toml` file to the main branch of your GitHub repo with whichever services you want to try out.
7. Try interacting with your repo, such as issuing `@rustbot` commands or interacting with PRs and issues (depending on which services you enabled in `triagebot.toml`). Watch the logs from the server to see what's going on.

#### Skipping workqueue loading

When triagebot starts, it eagerly loads the pull request workqueue for the `rust-lang/rust` repository, which can take up ~10-15 seconds. To disable this, for faster local experiments, pass the `SKIP_WORKQUEUE=1` environment variable to triagebot.

### Configure a database

To use Postgres, you will need to install it and configure it:

1. Install Postgres. Look online for any help with installing and setting up Postgres (particularly if you need to create a user and set up permissions).
2. Create a database: `createdb triagebot`
3. In the `.env` file, set the `DATABASE_URL`:

   ```sh
   DATABASE_URL=postgres://eric@localhost/triagebot
   ```

   replacing `eric` with the username on your local system.

### Configure webhook forwarding

I recommend at least skimming the [GitHub webhook documentation](https://docs.github.com/en/developers/webhooks-and-events/webhooks/about-webhooks) if you are not familiar with webhooks.
In order for GitHub's webhooks to reach your triagebot server, you'll need to figure out some way to route them to your machine.
There are various options on how to do this.
You can poke holes into your firewall or use a proxy, but you shouldn't expose your machine to the internet.
There are various services which help with this problem.
These generally involve running a program on your machine that connects to an external server which relays the hooks into your machine.
There are several to choose from:

* [gh webhook](#gh-webhook) — This is a GitHub-native service. This is the easiest to use.
* [ngrok](#ngrok) — This is pretty easy to use, but requires setting up a free account.
* <https://smee.io/> — This is another service recommended by GitHub.
* <https://localtunnel.github.io/www/> — This is another service recommended by GitHub.

#### gh webhook

The [`gh` CLI](https://github.com/cli/cli) is the official CLI tool which I highly recommend getting familiar with.
There is an official extension which provides webhook forwarding and also takes care of all the configuration.
See [cli/gh-webhook](https://docs.github.com/en/developers/webhooks-and-events/webhooks/receiving-webhooks-with-the-github-cli) for more information on installing it.

This is super easy to use, and doesn't require manually configuring webhook settings.
The command to run looks something like:

```sh
gh webhook forward --repo=ehuss/triagebot-test --events=* \
  --url=http://127.0.0.1:8000/github-hook --secret somelongsekrit
```

Where the value in `--secret` is the secret value you place in `GITHUB_WEBHOOK_SECRET` in the `.env` file, and `--repo` is the repo you want to test against.

#### ngrok

The following is an example of using <https://ngrok.com/> to provide webhook forwarding.
You need to sign up for a free account, and also deal with configuring the GitHub webhook settings.

1. Install ngrok.
2. Run `ngrok http 8000`. This will forward webhook events to localhost on port 8000.
3. Configure GitHub webhooks in the test repo you created.
   In short:

    1. Go to the settings page for your GitHub repo.
    2. Go to the webhook section.
    3. Click "Add webhook"
    4. Include the settings:

        * Payload URL: This is the URL to your Triagebot server, for example http://7e9ea9dc.ngrok.io/github-hook. This URL is displayed when you ran the `ngrok` command above.
        * Content type: application/json
        * Secret: Enter a shared secret (some longish random text)
        * Events: "Send me everything"

### Zulip testing

If you want a test Zulip instance, you can [register a free one](https://zulip.com/new/). To have your local triagebot talk to this Zulip instance you need to:
- Configure a [webhook forwarding service](#configure-webhook-forwarding)
- Create in your Zulip instance an outgoing webhook bot, binding it to the forwarding address created before.
- Launch your local triagebot setting `ZULIP_WEBHOOK_SECRET` to the webhook bot `token` value (you get that as part of the Zulip bot configuration)
- Set other Zulip env vars as needed (see example in `.env.sample`).

You can also simulate a Zulip webhook payload with `cURL`. For example, this is the payload sent to the triagebot server when tagging a Zulip bot in a stream.
``` sh
curl http://localhost:8000/zulip-hook \
    -H "Content-Type: application/json" \
    -d '{
        "data": "<CMD>",
        "token": "<ZULIP_WEBHOOK_SECRET>",
        "message": {
            "sender_id": <YOUR_ZULIP_ID>,
            "recipient_id": <BOT_ZULIP_ID>,
            "sender_full_name": "Randolph Carter",
            "sender_email": "r.carter@rust-lang.org",
            "type": "stream",
            "stream_id": 1234567,
            "subject": "Topic subject"
            }
        }'
```

Where:
- `CMD` is the full command you would issue on Zulip (ex. `@**triagebot** work show`)
- `ZULIP_WEBHOOK_SECRET`: can be anything. Must correspond to the env var `$ZULIP_WEBHOOK_SECRET` on your workstation
- `sender_*`: the Zulip user data sending the message. `sender_id` must be mapped to a GitHub user in this mapping: https://team-api.infra.rust-lang.org/v1/zulip-map.json
- `recipient_id`: Zulip ID of the recipient of the message (in this case the Zulip bot)

### Cargo tests

You can run Cargo tests using `cargo test`. If you also want to run tests that access a Postgres database, you can specify an environment variables `TEST_DB_URL`, which should contain a connection string pointing to a running Postgres database instance:

```bash
$ docker run --rm -it -p5432:5432 \
  -e POSTGRES_USER=triagebot \
  -e POSTGRES_PASSWORD=triagebot \
  -e POSTGRES_DB=triagebot \
  postgres:14
$ TEST_DB_URL=postgres://triagebot:triagebot@localhost:5432/triagebot cargo test
```

## License

Triagebot is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
