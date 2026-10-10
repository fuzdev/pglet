-- `isnull` is a TYPE_FUNC_NAME keyword: not a bare column name, quoted or not as a label.
-- @test
-- @expect-error: syntax error at or near "FROM" (42601)
-- @no-error-fields: the parser stops at `isnull` where 18.6's grammar stops at `FROM`
SELECT isnull FROM (SELECT 1 AS "isnull") s;
