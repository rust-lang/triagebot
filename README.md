# Triagebot

This is the triage and team assistance bot for the rust-lang organization.

Please see the [wiki] for our documentation, and feel free to contribute edits
if you find something helpful!

[wiki]: https://github.com/rust-lang/triagebot/wiki

## Installation

To compile the Triagebot you need OpenSSL development library to be installed (e.g. for Ubuntu-like Linux distributions `sudo apt install libssl-dev`).

Run `cargo build` to compile the triagebot.

The `GITHUB_WEBHOOK_SECRET`, `GITHUB_API_TOKEN` and `DATABASE_URL` environment
variables need to be set.

If `GITHUB_API_TOKEN` is not set, the token can also be stored in `~/.gitconfig` in the
`github.oauth-token` setting.

To configure the GitHub webhook, point it to the `/github-hook` path of your
webserver (by default `http://localhost:8000`), configure the secret you chose
in `.env`, set the content type to `application/json` and select all events.

## License

Triagebot is distributed under the terms of both the MIT license and the
Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.
