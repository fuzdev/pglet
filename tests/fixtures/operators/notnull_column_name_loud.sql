-- `notnull` is a TYPE_FUNC_NAME keyword: not a column definition's name.
-- @test
-- @expect-error: syntax error at or near "notnull" (42601)
-- @no-error-fields: both engines refuse the word; the reported token differs
CREATE TABLE nnc (notnull INT);
