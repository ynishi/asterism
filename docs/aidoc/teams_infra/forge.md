# teams-infra::forge

What the team's forge store keeps, whatever it keeps it in.

[`rows`] is the shape the SQLite adapter in
[`sqlite::forge`](crate::sqlite::forge) writes as columns. Nothing
here talks to a store, and nothing here knows what a team is —
taking a domain value apart and putting one back is the same work
whichever store is underneath, and the scope is the adapter's.

