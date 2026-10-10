-- `b_expr` keeps `IS [NOT] DISTINCT FROM` (see `default_b_expr_admits_a_comparison.sql`)
-- and drops the rest of the `IS` family, so `DEFAULT 5 IS NULL` is `42601` while the
-- `DISTINCT FROM` spelling one fixture over is accepted. That split is the whole
-- reason the mode tests the token *after* the `IS` rather than dropping the arm.
-- @expect-error: IS NULL is an a_expr production, excluded from a column DEFAULT
-- @test
CREATE TABLE def_isnull (a INT, b BOOL DEFAULT 5 IS NULL);
