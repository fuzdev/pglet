-- A `;` inside a `--` comment separates nothing, and a carriage return ends the comment,
-- so `SELECT 2` continues the first statement: a syntax error on both engines. A comment
-- that ran on to the next line feed would answer `SELECT 1`.
-- @test
-- @expect-error: syntax error at or near "2" (42601)
-- @no-error-fields: the parser stops at `SELECT` where 18.6's grammar stops at `2`, so the positions differ
SELECT 1 --c;SELECT 2;
