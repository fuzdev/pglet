-- A column constraint's `DEFAULT` takes `gram.y`'s restricted `b_expr`, which
-- **admits** the comparison operators and `IS [NOT] DISTINCT FROM`. A binding-power
-- threshold cannot express that — the excluded predicate band binds tighter than the
-- included comparisons — so this used to be `42601` here against 18.6's acceptance.
-- @setup
CREATE TABLE def_bexpr (k INT, b BOOL DEFAULT 5 = 4, c BOOL DEFAULT 5 IS DISTINCT FROM 4);
INSERT INTO def_bexpr (k) VALUES (1);
-- @test
SELECT k, b, c FROM def_bexpr;
