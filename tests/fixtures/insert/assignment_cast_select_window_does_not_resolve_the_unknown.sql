-- **A false-rejection control — do not trim it.** A window function in the select list
-- puts the query's target list on a `Plan::Window` rather than a `Plan::Project`, and a
-- window resolves no entry's `unknown` — 18.6 accepts this statement.
--
-- `Plan::Window` and `Plan::Aggregate` are the other two nodes that carry a query's own
-- target list 1:1, so each needs the same per-expression judgement a projection gets;
-- reading either off `Plan::output_columns` substitutes `text` for the bare `'t'` and
-- refuses.
-- @setup
CREATE TABLE acwi (id INT, b BOOL, i INT);
CREATE TABLE acwisrc (n INT);
INSERT INTO acwisrc VALUES (1), (2);
-- @test
INSERT INTO acwi (b, i)
SELECT 't', row_number() OVER (ORDER BY n) FROM acwisrc
RETURNING b, i;
