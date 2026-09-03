# asterism-teams-client::promotion

Handing an Asset over (#148 decisions 3, 4, 5, 7 and 8).

## One act, five steps, in this order

1. **Gather what cannot be re-derived** — the material's bytes, and
   the marks whose layer origin is `User`. That filter is decision
   4's and it lives in
   [`PromotedMark::gather`](crate::mapper::PromotedMark::gather);
   thumbnails, indexed bodies and `Imported`/`Machine` marks stay
   home because the receiving side can make them again.
2. **Ask what the team already has**, so a send can be skipped when
   the answer allows it — see the note on the have-check below.
3. **Enter the content against open work.** The team mints a
   `TeamAsset` for it, one per promotion (decision 7), so two
   members bringing identical bytes get one each and "who brought
   what" survives the second contributor. The work is the caller's
   to have opened: none is opened here, for the reason
   [`Promotion::pursuit_id`] gives.
4. **Push a round that names it**, with the entry id this client
   minted (decision 8) and the projection riding along (decision
   12). The content is there before the round names it, which is
   decision 5's ordering.
5. **Record the relation at home** — and only at home. The server
   holds no reference to the local Asset, in either direction.

The order is not an implementation detail. Content before round is
decision 5; link after both is what keeps a link row from ever
naming an entry that was never written.

## What v0 will promote

**One Asset, one local material.** Decision 3 says the team holds a
conversion and deliberately does not fix its composition — an Asset
may be several materials, or a Collection whose content is the
assets pointing at it. The teams plane's schema already admits
those: `team_asset.digest` is nullable precisely so a conversion
composed some other way can leave it empty.

What is missing is the composition itself — which parts land in the
CAS and which in rows — and that is a design question decision 3
leaves open rather than a gap in this function. So a Collection and
a multi-material Asset are **refused with a message that says
which**, rather than promoted as one of their parts. Promoting one
part of an Asset and calling it the Asset would be worse than
refusing: the receiving side could not reproduce it, and decision 4
is precisely the rule that what travels must be enough to.

## The have-check, honestly

Decision 19 adds it "to avoid re-sending". With the transport as
#151 built it, that saving is only partly available, and it is
worth stating rather than papering over.

The content verb is the only thing that mints a `TeamAsset`, and it
mints one from bytes — there is no verb that mints an asset over a
digest the team already holds. Decision 7 requires the mint on
every promotion, so a second member promoting identical bytes still
calls the verb and still sends the body.

What is available on the other axis is the one that matters day to
day: **a repeat of the same promotion sends nothing at all.** Before
anything is uploaded, [`promote`] asks the relation whether this
client already promoted this Asset onto this line, and a client
that did is answered from its own machine — without the have-check,
which is why [`PromotionOutcome::bytes_already_held`] is `None`
there. On the path that does send, the digest answer is reported so
a caller can show it, and the day a mint-over-held-digest verb
exists this function skips the body on it too.

## Functions

- `promote` — Runs a promotion.

## Types

- `Promotion` — One promotion, described.
- `PromotionOutcome` — What a promotion left behind, on both sides.

