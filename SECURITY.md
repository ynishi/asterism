# Security policy

Asterism is a local-first desktop application maintained by one person. This
document says where to send a vulnerability report, what happens after you do,
and what is in scope.

## Reporting

**Use GitHub's private vulnerability reporting on this repository** — the
"Report a vulnerability" button under the Security tab. It opens a private
thread visible only to the maintainer, so the report never sits in a public
issue while a fix is being written.

Please do not open a public issue for a suspected vulnerability. If private
reporting is unavailable to you for any reason, open a public issue that says
only that you have a security report and asks for a contact — no details.

What helps most in a report: what an attacker can reach, the steps to reproduce
it, and the version or commit you saw it on. A proof of concept is welcome but
not required.

## What happens next

This is a single-maintainer project, so these are commitments a single person
can keep rather than a team SLA:

| Stage | Target |
|---|---|
| Acknowledge the report | within 7 days |
| Initial assessment — is it a vulnerability, and how severe | within 30 days |
| Fix, or an explanation of why it will not be fixed | discussed with you once assessed |
| Public disclosure | when the fix ships, or 90 days after the report, whichever is first |

The 90-day deadline is the industry norm and it runs regardless of whether a
fix is ready, because an unfixed issue that stays secret indefinitely protects
nobody but the project. If you need it held longer — a coordinated release
with another project, for example — say so and it can be agreed.

Reports are handled through GitHub security advisories, which means you are
credited on the published advisory unless you ask not to be. There is no bug
bounty; nothing here is offered in exchange for a report.

## Scope

Asterism runs on the user's own machine and keeps their data there. In scope:

- the loopback HTTP API and the MCP endpoint the app serves — anything reachable
  from another process on the same machine, or from a page in a browser, that
  should not be;
- the import adapters, which parse files the user did not write (character
  cards, EXIF, media containers, session logs) — a malformed input that reaches
  code execution, path traversal, or a crash loop;
- the Tauri asset protocol scope, and any path by which the app reads or writes
  outside it;
- SQL injection, and data crossing between local profiles;
- a dependency with a known vulnerability that Asterism actually exercises.

Out of scope: anything that assumes the attacker already has code execution as
the user, or physical access to an unlocked machine — at that point the data is
theirs regardless. Also out of scope: files the user explicitly asked Asterism
to import or export, and the contents of their own database.

## Safe harbour

Good-faith research on your own installation is welcome and will not be met
with legal action. "Good faith" means: your own data, no third party's, no
attempt to reach anyone else's machine, and no public disclosure before the
window above closes.

## Handling a report that turns out to be a disclosure

If a report shows that protected content is already published — a credential in
history, personal data in a fixture — it stops being a vulnerability report and
follows [After a disclosure](PUBLIC_DEVELOPMENT.md#after-a-disclosure), which
is where the containment and notification steps live.
