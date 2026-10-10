-- The control for the null-test refusals: a bare select-list `$1` is not left untyped.
-- 18.6 resolves it to `text` once the select list is analysed, reports `text` in
-- `ParameterDescription`, and answers.
-- @param $1 text x
-- @test
SELECT $1 AS p;
