-- Positional walk, window/column-reference pair, polarity 1 of 2: the window
-- function is the left operand, so 18.6 answers `42P20 window functions are not
-- allowed in DEFAULT expressions`. The mirror is
-- `default_column_ref_before_window.sql` (`0A000`).
-- @expect-error: window before the column reference wins (42P20)
-- @test
CREATE TABLE dfc (a INT, b INT DEFAULT (row_number() OVER () + a));
