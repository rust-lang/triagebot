# Pull request assignment preferences backoffice

This is an administrative backoffice targeted at the Rust project contributors to set their preferences for pull request assignment.

When assigning the review of pull requests, this backoffice allows contributors to:
- set themselves on leave for any amount of time. During this time off contributors won't be assigned any new pull request
- set the maximum number of pull requests assigned to them
- set the desired number of days before a pull request assigned for review to the contributor might be considered for a reminder
- allow a flag to make their own preferences visible to all team members or only to team leaders and system administrators

This is a mostly static web page server-side generated, using the least amount possible of JavaScript.

This backoffice will set one cookie (`triagebot.session`) to understand if a user is already logged in. The cookie expires after
1 hour and is renewed at every access. The cookie is set to `Secure=true`, `HttpOnly` and `SameSite=Strict`.

Access authorization is handled by GitHub, so users will need to be logged in GitHub and authorize this Github Application.

Access to this backoffice is granted only to GitHub users that are members of a Rust project team (teams are defined [here](https://github.com/rust-lang/team/tree/HEAD/teams)). Only specific Rust teams are allowed to use
this backoffice (mostly for testing purposes, to switch users to the new workflow slowly). Teams allowed are defined in the env var `NEW_PR_ASSIGNMENT_TEAMS` (see `.env.sample`).

Teams members are expected to set their own review preferences using this backoffice. In case a team member didn't yet set their own preferences, these defaults will be applied:
- Max 5 pull requests assigned (see constant `PREF_MAX_ASSIGNED_PRS`)
- 20 days before a notification is sent for a pull request assigned that is waiting for review (see constant `PREF_ALLOW_PING_AFTER_DAYS`)

## How to locally run this backoffice

- Configure a webhook pointing to a local instance of the triagebot. Follow the instructions [in the README](https://github.com/rust-lang/triagebot#configure-webhook-forwarding).
- Configure a repository under your GitHub username and configure the same webhook URL in the "Webhooks" settings of the repository.
- Create a GiHub Application and configure the callback URL ([here](https://github.com/settings/apps)) pointing to your proxied triagebot backoffice using the path to the backoffice (ex. `http://7e9ea9dc.ngrok.io/github-hook/review-settings`) 
- Start your local triagebot: load the environment variable from a file (make a copy of `.env.sample`) and run `RUST_LOG=DEBUG cargo run --bin triagebot`

## TODO

- [ ] Handle cleanup of the preferences DB table for team members not existing anymore in the teams .toml: delete their assignments, PRs go back to the pool of those needing an assignee
- [ ] Cache somehow teams .toml download from github to avoid retrieving those `.toml` files too often
- [ ] maybe more input validation, see `validate_data()` in `./src/main.rs`
- [ ] Now we are handling contributors workload for a single team. Some contributors work across teams. Make this backoffice aware of other teams and show the actual workload of contributors


