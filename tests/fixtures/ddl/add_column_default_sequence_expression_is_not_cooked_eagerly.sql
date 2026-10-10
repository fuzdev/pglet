-- **A sequence-bearing default is exempt from the eager cook**, and the exemption is
-- not an optimization. An `ADD COLUMN` evaluates its default at DDL time (the sibling
-- `add_column_default_*_is_eager` files), but the cook's probe substitutes `1` for
-- every `nextval` whatever the counter holds — so cooking this expression eagerly
-- divides by zero at a value the statement never produces. Over an **empty** table
-- nothing is backfilled and no `nextval` is drawn at all: both engines accept, and the
-- counter still answers 1. The trailing `nextval` is the load-bearing half — an
-- `ALTER`-only `@test` would stay green over a cook that drew.
-- @setup
CREATE TABLE addcol_seqcook (id INT);
CREATE SEQUENCE addcol_seqcook_q;
-- @test
ALTER TABLE addcol_seqcook ADD COLUMN a INT DEFAULT 100 / (nextval('addcol_seqcook_q') - 1);
SELECT nextval('addcol_seqcook_q') AS n;
