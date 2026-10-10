-- Corresponding columns of the two sides must share a common type, else a loud 42804
-- (Postgres: "UNION types integer and text cannot be matched"). The explicit `::text`
-- makes the right side concretely typed, so both engines reach the type-mismatch error
-- rather than an unknown-literal coercion failure.
-- @expect-error: UNION of integer and text is a type mismatch
-- @test
SELECT 1 UNION SELECT 'x'::text;
