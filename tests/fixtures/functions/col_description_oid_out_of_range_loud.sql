-- An `int8` past the `oid` range resolves the overload and then fails its `oid`
-- conversion: `22003 OID out of range`.
-- @test
-- @expect-error: OID out of range
-- @expect-message
SELECT col_description(4294967296, 1);
