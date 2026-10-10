-- `AT TIME ZONE` is likewise excluded from `b_expr`, and it binds *tighter* than the
-- comparisons `b_expr` admits — which is why no single binding-power threshold can
-- separate the two and the grammar has to be a scoped mode.
-- @expect-error: AT TIME ZONE is excluded from a column constraint's DEFAULT
-- @test
CREATE TABLE def_atz (a INT, b TIMESTAMP DEFAULT now() AT TIME ZONE 'utc');
