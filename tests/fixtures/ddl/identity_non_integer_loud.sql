-- An identity column is backed by an implicit integer sequence, so Postgres
-- restricts its type to smallint/integer/bigint and rejects any other type —
-- SQLSTATE 22023 (invalid_parameter_value): `identity column type must be smallint,
-- integer, or bigint`. pglet matches rather than silently coercing nextval to text.
-- @expect-error: identity column type must be smallint, integer, or bigint
-- @test
CREATE TABLE t (id text GENERATED ALWAYS AS IDENTITY);
