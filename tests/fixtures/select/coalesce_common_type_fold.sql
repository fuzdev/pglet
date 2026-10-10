-- COALESCE resolves the common type of ALL its branches (Postgres's
-- `select_common_type`), not the first concrete one, and every branch is coerced to it.
-- Taking the first would *narrow*: `COALESCE(i8, i4)` would cast the bigint branch to
-- integer. The declared type is the assertion here — the rendered values agree either
-- way, so only the RowDescription tells the two apart.
-- Rows are arranged so a DIFFERENT branch wins on each row: with an unfolded type the
-- projection sends 4 bytes on one row and 8 on the next under a single declared width.
-- @setup
CREATE TABLE cf (id INT, a INT, b BIGINT, r REAL, n NUMERIC);
INSERT INTO cf VALUES (1, 7, NULL, NULL, NULL),
                      (2, NULL, 9, 2.5, 3.5),
                      (3, 4, 5, 1.5, 2.5);
-- @test
SELECT id,
       COALESCE(a, b) AS a_then_b,
       COALESCE(b, a) AS b_then_a,
       COALESCE(a, r) AS int_then_real,
       COALESCE(a, n) AS int_then_numeric,
       COALESCE(a, 1.5) AS int_then_numeric_literal
FROM cf ORDER BY id;
