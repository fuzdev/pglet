-- `JOIN` binds tighter than comma, so `FROM c, a JOIN b ON …` is two FROM items:
-- `c` and `(a JOIN b)`. The inner join's ON scope is only `a`/`b`, so referencing
-- the sibling comma item `c` fails loud (42P01) — matching Postgres, instead of
-- folding all three into one left-deep accumulator that lets the ON see `c`.
-- The `a`/`b` rows matter: pglet resolves the ON at eval time, so the inner
-- join must have at least one row pair to test for the out-of-scope `c.num` to
-- surface (Postgres rejects it at plan time regardless of data).
-- @setup
CREATE TABLE a (num INT);
CREATE TABLE b (num INT);
CREATE TABLE c (num INT);
INSERT INTO a VALUES (1), (2);
INSERT INTO b VALUES (1), (2);
INSERT INTO c VALUES (1), (2);

-- @test
-- @expect-error: invalid reference to FROM-clause entry for table "c" (42P01)
SELECT * FROM c, a JOIN b ON c.num = b.num;
