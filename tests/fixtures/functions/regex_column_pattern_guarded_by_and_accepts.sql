-- A pattern column is judged row by row, and a conjunct evaluated first keeps a row away
-- from it: `AND` stops at a `false` left operand, so row 3's `'a.c'` — a pattern pglet
-- refuses `0A000` wherever it is reached — is never reached, since `n < 3` excludes it.
-- A pattern the scan does reach is refused, pglet-stricter and loud
-- (`regex_column_pattern_loud.sql`).
-- @setup
DROP TABLE IF EXISTS rgx_and;
CREATE TABLE rgx_and (n INT, x TEXT, p TEXT);
INSERT INTO rgx_and VALUES (1, 'abc', '^ab'), (2, 'abc', '^b'), (3, 'abc', 'a.c');
-- @test
SELECT n FROM rgx_and WHERE n < 3 AND x ~ p ORDER BY n;
