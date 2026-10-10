-- The `SCHEMA` option is checkable with no extension state at all, because pglet
-- models exactly one schema: 18.6 answers `3F000 schema "nosuchschema" does not
-- exist` and so does this. It was accepted and dropped along with the rest of the
-- option tail.
-- @test
-- @expect-error: schema "nosuchschema" does not exist
CREATE EXTENSION pgcrypto SCHEMA nosuchschema;
