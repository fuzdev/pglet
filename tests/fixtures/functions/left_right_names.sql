-- The unaliased output column is the function's name, and `left` / `right` parse as
-- calls in every call spelling 18.6's grammar admits: the bare keyword before `(`, the
-- quoted name, and the schema-qualified one.
-- @test
SELECT left('abc', 1), right('abc', 1), "left"('xyz', 2) AS ql, pg_catalog.right('xyz', 2) AS qr;
