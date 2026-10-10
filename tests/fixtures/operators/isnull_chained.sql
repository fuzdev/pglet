-- The postfix tests chain with each other and with `IS`, as 18.6's do.
-- @test
SELECT NULL ISNULL ISNULL AS a, 1 NOTNULL IS NULL AS b, 1 ISNULL IS FALSE AS c;
