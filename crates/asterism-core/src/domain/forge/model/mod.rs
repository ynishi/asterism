//! The forge's model — the domain, and only the domain.
//!
//! Nothing here knows about storage, ports or transactions, and
//! nothing here may take its shape from them. A type that exists
//! because a table wanted it is a bug in this module, not a
//! convenience: the model is what the storage has to serve, and the
//! moment that runs backwards the schema starts deciding what the
//! product means.
//!
//! # The shape
//!
//! ```text
//!   Line = repository                  the forge's top entity
//!    ├ id / name / meta
//!    ├ strategy
//!    └ History
//!         │
//!         Genesis ──▶ ChangePoint ──▶ ChangePoint ──▶ …
//!                          │                        ▲ head
//!                          └ table : Entry ──▶ Row
//!                                              (existence, content, name)
//! ```
//!
//! A [`Line`] is a repository: one canonical history, and everything
//! that is on it derives from that history. A [`History`] is a chain
//! that begins at a [`Genesis`] and grows by one [`ChangePoint`] at a
//! time. Each change point carries a [`Table`] — one [`Row`] per entry
//! it moves, over three axes.
//!
//! # A line is the top
//!
//! Every rule the forge states is stated per line, so a line is the
//! largest thing it has an opinion about. Grouping lines is a question
//! about people — whose these are, who may see them, who may name them
//! — and answering it here would make the forge depend on identity,
//! which is the one dependency the boundary exists to refuse. Grouping
//! and ownership sit outside, with the model of teams and members that
//! holds them.
//!
//! What the line keeps is an identifier, because a line has to be
//! callable by something a person chose, and [`Name`] carries no claim
//! of uniqueness with it. "Unique among what?" needs an owner to
//! answer, and the owner is outside. Where there is only ever one
//! line, it is [`Line::ROOT`].
//!
//! # What is stored, and what derives
//!
//! **The history is the only record.** What is alive on a line, under
//! what name, at which content, is folded out of the chain by
//! [`states`] every time it is asked. Keeping a copy beside the
//! history would be a second thing to hold true, and the two would
//! disagree the first time a write went half-way.
//!
//! **The chain orders the history, not the clock.** A change point's
//! place is which node took it as a parent. Reading a timestamp
//! instead would be a second answer to a question the chain already
//! answers, and the two disagree the first time a clock steps
//! backwards — so no fold here looks at a time, and
//! [`History::land`] refuses anything that does not name the head.
//!
//! **A line's own description is a separate record.** Renaming a line
//! or changing how it settles collisions is not a change point: the
//! history says what happened to what the line carries, and that did
//! not. [`Meta`] holds those two stamps, and landing does not touch
//! them.
//!
//! # Nothing is removed
//!
//! Taking an entry off a line is a change point that says so — the
//! record stays, and so does the name and the content it had. That is
//! what makes a history worth keeping: everything that ever happened
//! is reachable, including what was dropped, including work that was
//! abandoned.
//!
//! So there is no operation here that deletes anything. Not a private
//! one, not a guarded one, not one that takes a flag. Absence is the
//! only way to mean it, because a delete path that exists gets called.
//!
//! # Where each rule lives
//!
//! The rules are held by types rather than checked by callers, which
//! is what stops them from being forgotten one call site at a time:
//!
//! - **A line always has a head.** [`History::begin`] mints the
//!   genesis, so [`History::head`] returns an id rather than an
//!   `Option`, and an untouched line is not a special case.
//! - **A history does not fork.** [`History::land`] refuses a change
//!   point whose parent is not the head.
//! - **A node is a genesis or a landing, never half of each.**
//!   [`Genesis`] and [`ChangePoint`] are separate types, so there is
//!   no pair of `Option` fields that must be empty together or filled
//!   together.
//! - **A line does not move to say nothing.** [`Table::of`] refuses an
//!   empty table.
//! - **A row can be read back.** [`Row::new`] refuses a row that
//!   states no axis, and one that takes an entry off the line while
//!   also naming or filling it.
//! - **A name is a name.** [`Name::new`] trims and refuses blank.
//! - **Two entries do not answer to one name.** [`History::land`]
//!   applies the whole table and asks of the result, so handing a name
//!   from one entry to another in one change point is legal and two
//!   live claims on it are not.
//!
//! # What this module depends on
//!
//! Four things outside it, and no more. Three are **boundary types** —
//! the vocabulary a third place owns because both sides need it:
//! `AssetId`, which [`Content`] wraps so nothing else here has to name
//! the layer below; attribution, which [`Act`] records; and the shared
//! error, which only [`error`] names. The fourth is the macro that
//! spells an id newtype, which shapes nothing.
//!
//! Every one of those imports is marked `SHARED KERNEL` at its `use`
//! line, tests included, so grepping that phrase lists every edge out
//! of this module that stays inside this crate. Nothing else reaches
//! out: no line of this module names storage, a port, a transaction,
//! or a type from the raw layer.
//!
//! Three third-party crates come too, and the grep does not list them
//! because they are not this crate's to hand over: `uuid`, `chrono`,
//! `thiserror`.
//!
//! # Refusals are the forge's own
//!
//! Everything here refuses in [`ForgeError`], which is the model's
//! vocabulary for what it will not accept, and it is folded into the
//! shared error once — at the edge, in [`error`]. A refusal named in
//! the shared vocabulary is one a caller cannot match on, and the
//! shape that follows from that is callers reading message text.
//!
//! Every rule the forge learns is added there, so what this model can
//! say no to stays readable in one place.
//!
//! # What is deliberately not here
//!
//! **The work log.** Rounds, operations and the fold from operations
//! to a table are the other half of the model, and a change point
//! names the pursuit it came out of without knowing anything else
//! about it.
//!
//! **The act that produces a change point.** Deciding one means
//! folding a pursuit's operations, normalising them against the head,
//! and settling collisions — it spans both logs, so it belongs with
//! the half that is missing rather than here.
//!
//! **Anything about people.** No owner, no persona, no actor set.
//! [`Act`] records who did a thing, because a history that cannot say
//! that is not a history; whether they were allowed to is a question
//! for the layer that knows what a person is.
//!
//! This module is where the model is written down. The reasoning that
//! produced it is on #63, and the work being built on it is #102 —
//! but neither is a place a reader has to go: what is true of the
//! model is stated here, beside the types that hold it.
//!
//! [`Line`]: line::Line
//! [`Line::ROOT`]: line::Line::ROOT
//! [`History`]: history::History
//! [`History::begin`]: history::History::begin
//! [`History::head`]: history::History::head
//! [`History::land`]: history::History::land
//! [`Genesis`]: history::Genesis
//! [`ChangePoint`]: history::ChangePoint
//! [`Table`]: table::Table
//! [`Table::of`]: table::Table::of
//! [`Row`]: table::Row
//! [`Row::new`]: table::Row::new
//! [`states`]: table::states
//! [`Name`]: value::Name
//! [`Name::new`]: value::Name::new
//! [`Act`]: act::Act
//! [`Meta`]: act::Meta
//! [`ForgeError`]: error::ForgeError

pub mod act;
pub mod error;
pub mod history;
pub mod line;
pub mod op;
pub mod pursuit;
pub mod table;
pub mod value;
