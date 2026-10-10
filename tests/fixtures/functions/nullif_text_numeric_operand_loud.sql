-- `NULLIF` over a `text` argument and a `double precision` one resolves no `=`
-- operator: `42883`, as on 18.6, decided over no row. The first argument was cast to
-- the second's type instead, so a `text` that reads as a number answered and one that
-- does not was the conversion's `22P02`.
-- @setup
CREATE TABLE t (tx TEXT, f FLOAT8);
-- @test
-- @expect-error: operator does not exist: text = double precision
-- @expect-message
SELECT nullif(tx, f) FROM t;
