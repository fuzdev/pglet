-- `~~*` (ILIKE) exists only over the string types too — narrower than LIKE, which
-- Postgres also registers for `bytea`. A uuid operand is 42883 either way.
-- @test
-- @expect-error: no uuid ILIKE operator
SELECT '11111111-1111-1111-1111-111111111111'::uuid ILIKE 'a%';
