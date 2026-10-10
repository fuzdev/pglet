-- A modifier beside a `[]` is the **element's**: `varchar(3)[]` is an array of
-- `varchar(3)`, and an explicit cast applies the cap element-wise (18.6's
-- `ArrayCoerceExpr`). It used to resolve to a bare scalar `varchar` — the `[]` went
-- with the modifier when the canonical name was split — and answered the input whole.
-- @test
SELECT '{abcdefg,xy}'::varchar(3)[] AS v;
