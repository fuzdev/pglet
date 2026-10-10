-- …and at the cast's own **operand** it outranks that column reference, for the same
-- reason: 18.6 reads the type modifier while transforming the `TypeCast` and never
-- resolves `nosuchcol`. Not at every position, though: where 18.6's parse analysis reaches
-- a column reference *before* the cast, the column wins there and not here — `SELECT
-- nosuchcol, 'x'::char(0) FROM t` is `42703` against this `22023` — the pre-existing
-- cast-position divergence, `EDGES.md` row 119's first residual, which records the
-- positions probed.
-- @setup
CREATE TABLE rtmc (x INT);
-- @expect-error: the length refusal outranks the undefined column
-- @expect-message
-- @test
SELECT nosuchcol::char(0) FROM rtmc;
