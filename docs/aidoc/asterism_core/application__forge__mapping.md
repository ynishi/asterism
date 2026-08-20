# asterism-core::application::forge::mapping

Wire parsing for the forge's own ids.

Two functions, and they are here rather than in
[`application::mapping`](crate::application::mapping) for one
reason: that module is the raw layer's, and a forge id parsed there
made the raw layer name a forge type (#81). Nothing but the forge's
services ever called them.

The uuid reading itself stays shared — [`parse_uuid`] is about
uuids, not about either side, and two copies of "is this a uuid"
would be two error messages for one mistake.

## Functions

- `parse_project_id` — Parses the wire representation of a project id.
- `parse_pursuit_id` — Parses the wire representation of a pursuit id.

