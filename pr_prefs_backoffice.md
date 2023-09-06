# Pull request assignment preferences backoffice

This is an administrative backoffice targeted at the Rust project contributors to set their preferences in terms of pull request assignment.

When assigning the review of pull requests, this backoffice allows contributors to:
- set themselves on leave for any amount of time. During this time off contributors won't be assigned any new pull request
- set the maximum number of pull requests assigned to them
- set the desired number of days before a pull request assigned for review to the contributor might be considered for a reminder
- allow a flag to make their own preferences visible to all team members or only to team leaders and system administrators

This is a mostly static web page server-side generated, using the least amount possible of JavaScript.

This backoffice will set one cookie (`triagebot.session`) to understand if a user is already logged in. The cookie expires after
1 hour and is renewed at every access. The cookie is set to `Secure=true`, `HttpOnly` and `SameSite=Strict`.

Access authorization is handled by GitHub, so users will need to be logged in GitHub and authorize this Github Application.

Access to this backoffice is granted only to GitHub users that are members of a Rust project team (initially: the compiler team, then others).

## How to locally run this backoffice

- Configure a webhook pointing to a local instance of the triagebot. Follow the instructions [in the README](https://github.com/rust-lang/triagebot#configure-webhook-forwarding).
- Configure a repository under your GitHub username and configure the same webhook URL in the "Webhooks" settings of the repository.
- Create a GiHub Application and configure the callback URL [here](https://github.com/settings/apps) pointing to your proxied triagebot backoffice using the path to the backoffice (ex. `http://7e9ea9dc.ngrok.io/github-hook/review-settings`) 
- Start your local triagebot: load the environment variable from an .env file (make a copy of `.env.sample`) and run `RUST_LOG=DEBUG cargo run --bin triagebot`

## TODO

- [ ] Figure out the case of someone reassigning a PR because the assignee does not respond. Pick the next one in the generated list?
- [ ] Handle team members not exiting anymore in the .toml source file (have a cronjob (?) to delete their assignments from the DB and go back to the pool)
- [ ] Cache somehow team members from github in order to now get thos .toml files at every request
- [ ] maybe more input validation, see `validate_data()` in `./src/main.rs`
- [ ] Now we are handling contributors workload for a single team (compiler). But some contributors work across teams. Make this backoffice aware of other teams and show the actual workload of contributors


