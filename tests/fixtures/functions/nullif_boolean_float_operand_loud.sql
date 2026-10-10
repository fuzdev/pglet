-- `NULLIF` over a `boolean` and a `double precision` resolves no `=` operator: `42883`,
-- as on 18.6, where the first argument's cast to the second's type was `42846`.
-- @setup
CREATE TABLE t (b BOOL, f FLOAT8);
INSERT INTO t VALUES (true, 1.5);
-- @test
-- @expect-error: operator does not exist: boolean = double precision
-- @expect-message
SELECT nullif(b, f) FROM t;
