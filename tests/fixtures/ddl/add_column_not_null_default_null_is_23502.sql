-- `ALTER TABLE … ADD COLUMN c INT NOT NULL DEFAULT NULL` over a non-empty table is
-- `23502` on 18.6: `column "c" of relation "…" contains null values`, raised in
-- `ATRewriteTable` once the backfilled rows exist. `DEFAULT NULL` is not a value the
-- rewrite can put in a NOT NULL column, because 18.6 stores no `pg_attrdef` row for a
-- NULL-constant default at all — it *is* "no default".
--
-- pglet used to accept this and write a NULL into every existing row of the NOT NULL
-- column, because the gate asked whether a DEFAULT was present rather than what it
-- resolved to. It now tests the resolved backfill values.
--
-- The row-survival half of this — that the refusal leaves the rows and the catalog
-- untouched — is `crates/pglet/tests/alter_add_column_backfill.rs`'s
-- `add_column_not_null_default_null_on_nonempty_is_23502`: `@expect-error` compares
-- SQLSTATEs, so it is green for a refusal raised after the writes.
-- @expect-error: NOT NULL column cannot be backfilled with NULL
-- @setup
CREATE TABLE ancd (id INT);
INSERT INTO ancd VALUES (1), (2);
-- @test
ALTER TABLE ancd ADD COLUMN c INT NOT NULL DEFAULT NULL;
