-- An expression key part is named as a `SELECT` would name that expression's output
-- column (`FigureIndexColname`), with `expr` where a `SELECT` says `?column?`: a function
-- call is the function's name, a cast over a column is the column's, an operator is
-- `expr` — one index each, so each name is read alone.
-- @setup
CREATE TABLE ciu3 (a INT, t TEXT, j JSONB);
CREATE INDEX ON ciu3 (lower(t));
CREATE INDEX ON ciu3 ((a + 1));
CREATE INDEX ON ciu3 ((a::bigint));
CREATE INDEX ON ciu3 ((j ->> 'k'));
CREATE INDEX ON ciu3 (coalesce(a, 0));
CREATE INDEX ON ciu3 ((abs(a)::text));
CREATE INDEX ON ciu3 ((a IS NULL));
-- @test
SELECT indexname, indexdef::text AS indexdef FROM pg_indexes
WHERE schemaname = 'public' AND tablename = 'ciu3' ORDER BY indexname;
