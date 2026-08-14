# asterism-infra::sqlite::repo::app_setting

SQLite adapter for the `AppSettingRepository` port.

Stores only the keys a user has actually overridden; absence of a row
is the "use the code default" state, so `delete` is the reset path.

**Downgrade tolerance.** A row this build cannot interpret — an
unknown key, or a timestamp outside the representable range — is
treated as "not overridden" rather than as an error. A profile opened
by a newer build can carry keys this one has never heard of, and a
settings screen that refuses to render at all would be a far worse
outcome than ignoring one row. The row is left on disk, so going back
to the newer build restores the preference, and every skip is logged
at warn level so the anomaly is visible rather than silent.

`list` and `find` apply the *same* rule on purpose: if `find` raised
an error where `list` skipped, one uninterpretable row would make the
two read paths disagree about whether a key is overridden.

## Types

- `SqliteAppSettingRepository` — SQLite adapter for `AppSettingRepository` (uses a writer isle).

