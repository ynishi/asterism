# asterism-infra::memory

In-memory adapters — the ports satisfied without a database.

These are not test doubles in the usual sense. A double stands in
for a thing; these are the thing, built over a `Mutex<Vec<Row>>`
instead of over SQLite. What makes that worth having is what it
forces: a store keeps rows, so an adapter here decomposes a domain
value on the way in and rebuilds it on the way out, and the rebuild
goes through the same door a real one will.

The alternative — a fake that keeps the domain objects themselves —
satisfies the same traits and answers the same calls, and never
once meets the question the read half exists to ask. A service
passing against one of those has been tested against a `HashMap`
wearing a port's name.

