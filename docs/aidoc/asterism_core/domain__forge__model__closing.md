# asterism-core::domain::forge::model::closing

Ending work — the one act that moves both logs.

```text
  close(&Line, &Pursuit, outcome, act)
           │
           ├── Abandoned ────────────────► Closing::Abandoned { close }
           │
           └── Satisfied ── normalise ──► write set ──► collisions
                                │             │             │
                            (nothing        (empty)      (any)
                             refuses)          │             │
                                │           refuse        refuse
                                ▼
                       Closing::Landed { close, point }
```

Everywhere else, a decision moves one log. Opening, passing and
taking something in write only to the pursuit; renaming a line
writes only to its own description. This is the exception, and it
is the only one: ending work as satisfied puts a change point on
the line, and the two are one act rather than two that happen to
run together.

# Both, or neither

[`Closing`] holds the close and the change point in one value with
private fields. There is no order between them, no window where one
is written and the other is not, and no way to hold either alone —
`Close::new` and `ChangePoint::new` are both closed to the model,
and outside tests this is the only function that calls them.

When [`close`] refuses, neither node exists. Nothing was minted, so
there is nothing to undo and nothing to compensate for later. That
is what "one act" has to mean in a type: not that two writes are
coordinated, but that there is one thing that either happened or
did not.

# Deciding is not applying, and neither is storing

[`close`] returns what would be born and touches nothing. Putting
it on the two logs in memory is [`Closing::apply`]. Keeping it is
somebody else's problem, and the port that does it takes this one
value, so there is no second call for a caller to forget.

Splitting it this way is what makes the decision testable against
any pair of a line and a pursuit, without a store and without a
clock: the same inputs give the same answer, and the answer is a
value rather than a mutation somebody has to go looking for.

# This function does not settle anything

A collision is refused here, never resolved. Turning one into a
divergence writes a pass into the pursuit, under the line's
strategy, and it happens while the work is open — by the time
anybody is closing, the question has been settled or it has not.

Closing is where that is checked and nowhere else, which is what
keeps the check from being a flag somebody has to clear: collisions
are computed from the two logs whenever they are asked, so what is
refused here is the state the logs are actually in.

## Functions

- `close` — Ends work, and says what that puts on the line.

## Types

- `Closing` — What ending work produced.

