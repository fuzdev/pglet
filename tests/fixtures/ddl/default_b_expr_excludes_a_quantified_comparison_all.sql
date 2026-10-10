-- The `ALL` spelling of `default_b_expr_excludes_a_quantified_comparison.sql`. Same
-- production, and worth its own row because the parser reads the three quantifier
-- keywords through one predicate — a gate written for `ANY` alone would pass here.
-- @expect-error: ALL is the same excluded production as ANY
-- @test
CREATE TABLE def_quant_all (a INT, b BOOL DEFAULT 5 = ALL(ARRAY[1,2]));
