# teams-server::oidc

Sign-in through the instance's identity provider (#163): the
attempts a desktop app starts, the pages a browser walks, the
loopback hand-back that ties the answer to the machine the app runs
on, and the collect that turns a provider's answer into an ordinary
session.

## The shape

```text
app ──► listens on http://127.0.0.1:<port>
app ──► POST /teams/auth/oidc/attempts {collector, label, loopback_port}
                                       ──► {attempt_id, start_url}
app ──► opens start_url in the system browser
  browser ──► GET  …/attempts/{id}            the page: "sign in <label>?"
  browser ──► POST …/attempts/{id}/authorize  303 to the provider
  browser ──► provider ──► GET …/callback?code&state
                                               exchange, verify, resolve
  browser ◄── 303 http://127.0.0.1:<port>/…?attempt={id}&grant=…
  browser ──► the app's listener ──► 303 …/attempts/{id}/done  (a page)
app ──► POST …/attempts/{id}/collect {secret, grant} ──► session, once
```

The listener's two lines are the contract the app is to meet, not
something this crate holds; the wire crate's `OidcAttemptDto` is
where it is stated for the app.

From the session on, nothing is new: the app mints a device token
on it the way it does after a password (#204), and the gate never
learns which way in was taken.

## What binds the answer to the app, and to its machine

Two things, for two attackers.

The attempt id travels through a browser's history and a provider's
logs, so it is not what collects the session. The app keeps a secret
and starts the attempt with its SHA-256; the collect presents the
secret, and a collect that presents anything else is answered as
though the attempt did not exist. That is one answer for six cases
— a wrong secret, a wrong grant, an id nothing names, an attempt
past its expiry, one already collected, one the browser has not
finished — so that none of them can be told apart from outside.
That closes the case of a third party who *learns* an id.

It closes nothing against somebody who *started* the attempt and
gets a person to finish it in their browser — the shape device-code
phishing takes — because the starter holds the secret. What closes
that case is where the provider's answer goes: not to the app's
poll, but to the browser, as a redirect to the loopback address the
attempt was started with, carrying a one-time grant the collect
also requires. The browser that finished the sign-in is on the
person's machine, `127.0.0.1` on that machine is the person's
machine, and an app listening there is the app the person is
running. An attempt started elsewhere sends its grant to a port on
the victim's machine where nothing of the attacker's is listening,
and the grant is never collected. There is no poll to fall back to;
a client that cannot listen on loopback cannot sign in this way,
which is the price, stated. RFC 8252 §7.3 is the loopback shape,
and the AWS CLI's move from device code to loopback is the
precedent for choosing it over polling for exactly this attack.

The page before the provider is still there and still asks, with
the label the attempt was started with. It is a courtesy and a
speed bump, not the defence: the label is text whoever started the
attempt typed. What the page does have to do is not be skippable —
its button takes a token only the page hands out, and the page
refuses to be framed — so that the person does see it.

## In memory, not in the database

Attempts live in a map on the context, swept on every start and
gone with the process. An attempt is ten minutes of state with no
meaning after the session it produced, and a restart mid-sign-in
costs the person a second click, which is the same cost the
limiter's in-memory buckets accept for the same reason.

## Types

- `Collect` — What a collect comes to, before it is a status code.
- `OidcSignIn` — The provider half of the context: the client, the bindings, and the

## Constants

- `ATTEMPT_TTL_MS` — How long an attempt may sit between being started and being

