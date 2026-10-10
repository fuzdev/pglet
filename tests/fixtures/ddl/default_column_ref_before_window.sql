-- Positional walk, window/column-reference pair, polarity 2 of 2 — the mirror of
-- `default_window_before_column_ref.sql`. The column reference is reached first, so
-- 18.6 answers `0A000` rather than the window function's `42P20`.
-- @expect-error: column reference before the window wins (0A000)
-- @test
CREATE TABLE dfd (a INT, b INT DEFAULT (a + row_number() OVER ()));
