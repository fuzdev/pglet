-- `scan.l`'s trailing `+`/`-` rule: the sign stays part of a multi-character operator
-- when the run holds a character SQL's own operators do not use, so `2^-1` names the
-- operator `^-` rather than `2 ^ (-1)`.
-- @test
-- @expect-error: operator does not exist: integer ^- integer
-- @expect-message
SELECT 2^-1;
