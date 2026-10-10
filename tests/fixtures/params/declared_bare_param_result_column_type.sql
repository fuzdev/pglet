-- A declared `$N` projected bare, and under an operator or a function with no sibling
-- the lowering types, is a result column of the type 18.6 resolves from the declared
-- one — not `text`, not the other operand's type, not `float8`. `EDGES.md` row 145.
-- @param-declared $1 int8 -5
-- @test
SELECT $1 AS bare, $1 + 1 AS plus_one, abs($1) AS absolute, -$1 AS negated, +$1 AS plus;
