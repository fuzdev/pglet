-- GROUP BY over a numeric column folds display-scale variants: `1.5`, `1.50`, and
-- `1.500` are ONE group (Postgres), matching DISTINCT. Regression guard that the
-- numeric grouping key uses the order key, not the differing text form.
-- @setup
CREATE TABLE nn (id INT, v NUMERIC);
INSERT INTO nn VALUES (1, 1.5), (2, 1.50), (3, 1.500), (4, 2.5), (5, 2.50);
-- @test
SELECT count(*) AS n FROM nn GROUP BY v ORDER BY n;
