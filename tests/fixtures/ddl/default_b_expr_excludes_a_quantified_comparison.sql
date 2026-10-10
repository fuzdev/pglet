-- A **quantified comparison** (`= ANY (…)` / `ALL` / `SOME`) is an `a_expr`
-- production only — `gram.y`'s `a_expr subquery_Op sub_type …` has no `b_expr`
-- twin — so a column constraint's `DEFAULT` stops at the quantifier keyword, before
-- its operand is looked at. It is the fourth arm the mode has to consult and the one
-- the first pass omitted: the `NOT`, `IS`-family, predicate-band and `AT TIME ZONE`
-- arms were gated and this one was not, so the statement was accepted here and
-- `42601` on 18.6. `ALL` and `SOME` are the same production and the same answer.
-- @expect-error: a quantified comparison is excluded from a column constraint's DEFAULT
-- @test
CREATE TABLE def_quant (a INT, b BOOL DEFAULT 5 = ANY(ARRAY[1,2]));
