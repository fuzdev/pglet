-- The **silent** member of the class its `'abc'::text` sibling covers loud, and the one
-- the value-coercion cook could not see: `'5'::text` is `text` to the assignment cast
-- (`42804` on 18.6) and parses cleanly as an integer, so pglet's value probe accepted it
-- and defined a column whose default 18.6 refuses outright. Eleven shapes of this kind
-- were measured across a 456-shape sweep — `int2`/`int4`/`int8`/`float4`/`float8`/
-- `numeric`/`interval`/`jsonb` from `'5'::text`, and `bytea` from `'abc'::text`,
-- `'5'::text` and `concat('a','b')`.
-- @expect-error: a typed text default that happens to parse is still a type mismatch (42804)
-- @test
CREATE TABLE dsa (x INT DEFAULT '5'::text);
