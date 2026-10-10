-- A `VALUES` list's `text` column beside a computed `integer` branch has no common type:
-- `42804`, as on 18.6. The fold answered the `text` while the column was not a typed
-- branch and the computed sibling was cast to it.
-- @setup
CREATE TABLE t (i INT);
INSERT INTO t VALUES (1);
-- @expect-error: COALESCE types text and integer cannot be matched
-- @test
SELECT COALESCE(v.x, t.i + 0) FROM t, (VALUES ('7')) v(x);
