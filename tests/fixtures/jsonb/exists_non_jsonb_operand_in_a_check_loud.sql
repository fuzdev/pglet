-- The stored-expression position of the same rule. It is **not** a discriminating guard
-- on the DDL-time walker's arm: the value probe beside that walk evaluates the CHECK over
-- a representative row and the eval-time gate refuses the pair there too, so this stays
-- red only while at least one of the three callers consults the rule. What it does assert
-- is the thing that matters at this position — the constraint is refused rather than
-- **recorded**, which is what would make the catalog dump to SQL stock 18.6 refuses.
-- @test
-- @expect-error: no integer ? integer operator in a CHECK
CREATE TABLE jsonb_check_operand (a INT CHECK (a ? 1));
