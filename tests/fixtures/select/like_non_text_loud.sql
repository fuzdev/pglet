-- Postgres registers `~~` (LIKE) only over the string types, so a non-text left
-- operand resolves to no operator (42883). pglet used to return `false` for every
-- row instead — a silent wrong answer, not an error.
-- @test
-- @expect-error: no integer LIKE operator
SELECT 1::int LIKE '1%';
