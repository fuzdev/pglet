-- The same pair under `greatest`: `upper(tx)` beside a computed `double precision`
-- branch is not converted to it, and the set is refused as 18.6 refuses it.
-- @setup
CREATE TABLE t (k INT, tx TEXT, f FLOAT8);
INSERT INTO t VALUES (1, '7', 7), (2, 'x', 2.5);
-- @expect-error: GREATEST types text and double precision cannot be matched (42804)
-- @test
SELECT k, greatest(upper(tx), f + 0) FROM t ORDER BY k;
