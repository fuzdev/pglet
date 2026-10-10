-- The control for `add_column_not_null_backfill_beats_check.sql`: drop the `NOT NULL`
-- and the identical statement is `23514` (`check constraint … is violated by some row`)
-- on both engines, because the NULL backfill really does violate `c IS NOT NULL`.
--
-- Without this pair the `23502` fixture beside it is consistent with an engine that
-- never evaluates an inline CHECK over the backfilled rows at all.
-- @expect-error: the NULL backfill violates the inline check
-- @setup
CREATE TABLE abcn (id INT);
INSERT INTO abcn VALUES (1), (2);
-- @test
ALTER TABLE abcn ADD COLUMN c INT CHECK (c IS NOT NULL);
