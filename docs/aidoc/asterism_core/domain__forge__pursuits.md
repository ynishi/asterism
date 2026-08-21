# asterism-core::domain::forge::pursuits

Keeping work logs, stated in the forge's own words.

[`Pursuits`] is the other face the forge asks for, and it is the
quiet one: opening work and adding passes to it are what happens
most, and neither touches a line.

# A pass names the node it sits on

[`Pursuits::push`] takes the node the caller believes the work ends
at, beside the pass it wants to add. The model refuses a pass that
does not sit on the head, but it judges the work log it was
*given*, which is the log as it was when it was read. Naming the
head makes the write itself conditional, so two passes written at
once cannot both land and the loser is told.

# It cannot end work

There is no close here. Ending work as satisfied puts a change
point on a line, and the two are written together — that call is
[`Closings::commit`]. Abandoning goes the same way rather than
getting a shortcut of its own: one door for endings means a reader
of this trait cannot find a second one.

# Listing hands back whole pursuits, and that is a bet

[`Pursuits::of_line`] and [`Pursuits::children`] return every pass
of every pursuit they answer with, which is more than a caller
showing a list needs. There is no lighter shape because the model
has no half-pursuit, and inventing one for a listing would put a
read's convenience inside the model.

The bet is that a line does not accumulate work faster than
somebody can read about it. If that turns out false, what arrives
is a summary the transport asks for with a measurement behind it —
not a guess made here.

# Reading gives back the whole pursuit

Adding a pass checks the head, deciding a close folds every pass,
and both are rules the model holds about the chain. Handing back
less would move them to whoever answers this call.

[`Closings::commit`]: crate::domain::forge::closings::Closings::commit

## Traits

- `Pursuits` — Keeps work logs.

