-- `ALTER TABLE … ADD COLUMN`'s inline `DEFAULT` is the *same* `ColConstraintElem`
-- production as `CREATE TABLE`'s, so the quantified comparison is `42601` there too.
-- The route matters: the mode is set per caller, and gating only the `CREATE TABLE`
-- one leaves this accepted.
-- @setup
CREATE TABLE add_quant (a INT);
-- @expect-error: a quantified comparison is excluded from an ADD COLUMN's inline DEFAULT
-- @test
ALTER TABLE add_quant ADD COLUMN b BOOL DEFAULT 5 = ANY(ARRAY[1,2]);
