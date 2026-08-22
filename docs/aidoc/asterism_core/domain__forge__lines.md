# asterism-core::domain::forge::lines

Keeping lines, stated in the forge's own words.

[`Lines`] is one of the four faces the forge asks for. Every type it
mentions — [`Line`], [`Name`], [`Act`] — belongs to the model, so
nothing here has to be translated and there is no vocabulary to
isolate. Whatever implements it is somebody else's problem, and the
forge never names them.

# A line moves through one door, and it is not this one

There is no `record` here. A change point exists because a pursuit
was satisfied, and the two are written together or not at all — so
the call that moves a line is [`Closings::commit`], where both
halves are in hand. A second way to append here would be a way to
move a line without ending any work, which is the state the model
has no word for.

What remains is what a line is when nothing is being closed:
opening it, reading it, and moving its own description.

# Reading gives back the whole line

[`Lines::get`] answers with the history included, because the rules
the model holds are about the chain — deciding a close folds it,
and recording checks its head. Handing back less would move those
rules to whoever answers this call, and there would be as many
copies of them as there are implementations.

# Nothing removes part of a line, and one call removes all of it

No truncate, no rewrite of a recorded node, no delete of anything
inside a history — the same absence the model has, for the same
reason: everything that ever happened stays reachable, and a path
that exists gets called. Renaming a line, changing its strategy and
archiving it are here because they move a line's own description,
which is a record the history does not keep.

[`Lines::discard`] is the exception, and it is whole-line or
nothing. A line that holds bytes somebody else may want back has to
be lettable-go of somehow, and the model's answer is that the way
out is the whole way out: archive it, end the work against it, and
then it goes with everything under it. Half of it going is the
state that would leave a history unreadable, which is what every
other absence here is protecting.

# The error is the shared one

The model refuses in its own vocabulary, but a port is where
failures from outside arrive — a line that is not there, a store
that is unreachable — and those have no forge-side name. So the
port speaks [`DomainError`], which the model's own refusals convert
into at a single edge.

[`Line`]: crate::domain::forge::model::line::Line
[`Name`]: crate::domain::forge::model::value::Name
[`Act`]: crate::domain::forge::model::act::Act
[`Closings::commit`]: crate::domain::forge::closings::Closings::commit

## Traits

- `Lines` — Keeps lines.

