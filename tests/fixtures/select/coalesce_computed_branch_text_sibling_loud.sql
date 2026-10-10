-- A computed `integer` branch beside a `text` column has no common type: `42804`, before
-- any row, as 18.6 refuses it.
-- @setup
CREATE TABLE t (i INT, tx TEXT);
INSERT INTO t VALUES (NULL, 'b');
-- @test
-- @expect-error: COALESCE types integer and text cannot be matched
SELECT COALESCE(i + 0, tx) FROM t;
