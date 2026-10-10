-- Positional walk, `$N`/column-reference pair, polarity 2 of 2 — the mirror of
-- `default_parameter_before_column_ref.sql`. The column reference is reached first,
-- so 18.6 answers `0A000` rather than `42P02`.
-- @expect-error: column reference before the parameter wins (0A000)
-- @test
CREATE TABLE dfh (a INT, b INT DEFAULT (a + $1));
