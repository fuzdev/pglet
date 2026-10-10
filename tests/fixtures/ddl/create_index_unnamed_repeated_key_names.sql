-- A key part whose name an earlier part of the same key already took gets a digit of its
-- own before the names are joined (`ChooseIndexColumnNames`): a repeated column is `a`,
-- `a1`, `a2`; two operator parts are `expr`, `expr1`; a repeated function `lower`,
-- `lower1`. The digit counts per name, and it is not the digit a taken *index* name gets.
-- @setup
CREATE TABLE ciu5 (a INT, b INT, t TEXT);
CREATE INDEX ON ciu5 (a, b, a, a);
CREATE INDEX ON ciu5 ((a + 1), (a + 2));
CREATE INDEX ON ciu5 (lower(t), upper(t), lower(t));
-- @test
SELECT indexname FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu5' ORDER BY indexname;
