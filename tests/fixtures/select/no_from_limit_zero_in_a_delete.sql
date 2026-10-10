-- **The write polarity of the silent half.** No statement here raises on *either* engine:
-- the no-FROM `LIMIT 0` was discarded, the subquery answered its row, and the `DELETE`
-- removed a row 18.6 keeps. Same SQL, no error anywhere, different rows — which is what
-- separates this class from the sort-key members beside it, every one of which is loud on
-- 18.6.
--
-- The `@setup` runs the write and the `@test` reads the survivors, so the assertion is the
-- surviving rows rather than a SQLSTATE. Its `UPDATE` sibling is
-- `select/no_from_offset_in_an_update.sql` and its `INSERT … SELECT` sibling
-- `insert/select_no_from_limit_zero_writes_nothing.sql`.
-- @setup
CREATE TABLE lzw (a INT);
INSERT INTO lzw VALUES (1), (2), (3);
DELETE FROM lzw WHERE a IN (SELECT 1 AS q LIMIT 0);

-- @test
SELECT a FROM lzw ORDER BY a;
