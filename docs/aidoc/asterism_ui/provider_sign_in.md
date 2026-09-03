# asterism-ui::provider_sign_in

The desktop's half of a sign-in through the team's identity
provider (#163): the secret the server sees only as a hash, and the
loopback listener the browser is sent back to.

The server's `oidc` module states the shape and what each leg
closes; this is the app's end of it. The app makes a secret and
starts an attempt with its SHA-256 and a port it is listening on at
`127.0.0.1`. The person signs in in their browser. The server's
callback sends that browser to this port with a one-time grant —
or with `refused=1` — and the listener answers with a `303` to the
server's done page, so the tab ends on the server saying what
happened rather than on this process. The app then collects the
session with the secret and the grant together.

**Why loopback and not a poll** is the server's argument, and the
one sentence of it that matters here: the grant lands on a port of
the machine the browser is on, so an attempt somebody else started
delivers its grant to a port on *their* victim's machine where
nothing of theirs is listening. This listener is what makes that
true, which is why it binds `127.0.0.1` and nothing wider, answers
one attempt and nothing else, and goes away the moment it has
answered.

The listener is an `axum` router on an ephemeral port — the same
server the app already runs for its own local serve, so no new
dependency — serving exactly one route for exactly one request.

## Types

- `Listener` — A listener bound and waiting, before the attempt exists.
- `Outcome` — What the browser brought back.
- `Secret` — The secret an attempt is bound to, and the hash the server is told.
- `Waited` — Why a wait ended without an answer.

