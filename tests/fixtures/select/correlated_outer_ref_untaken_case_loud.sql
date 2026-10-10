-- An unparseable literal against a correlated outer reference in an **untaken** `CASE`
-- branch. Postgres folds a constant coercion at parse time, so the error is
-- independent of which arm would run; pglet's `validate_const_casts` reproduces that —
-- but only for a literal that *has* a cast, and an unresolved correlated reference
-- gets none.
--
-- So this shape was **silently wrong** before the chaining, not merely over-rejected:
-- the branch was never evaluated, the bad literal was never parsed, and the query
-- returned a row where Postgres raises `22P02`. It is the strictest statement of the
-- eager-error invariant the chaining had to preserve — deferring resolution must not
-- make an error conditional on evaluation order.
-- @setup
CREATE TABLE cc_o (okey INT, ouid UUID);
CREATE TABLE cc_i (ikey INT);
INSERT INTO cc_o VALUES (1, '11111111-1111-1111-1111-111111111111');
INSERT INTO cc_i VALUES (1);
-- @expect-error: unparseable constant in an untaken CASE branch — 22P02, eagerly
-- @test
SELECT okey FROM cc_o WHERE EXISTS (SELECT 1 FROM cc_i WHERE CASE WHEN 1 = 0 THEN cc_o.ouid = 'not-a-uuid' ELSE true END);
