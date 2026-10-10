-- A `JOIN … USING` column merged from same-typed columns stays certain through a derived
-- table and a `WITH` query that pass it on, by name or by `*`.
-- @setup
CREATE TABLE a (k INT, v INT);
CREATE TABLE b (k INT, w INT);
INSERT INTO a VALUES (1, 10), (4, 20);
INSERT INTO b VALUES (1, 100), (4, 200);
-- @test
WITH c AS (SELECT * FROM a JOIN b USING (k))
SELECT (SELECT avg(k) FROM (SELECT k FROM a JOIN b USING (k)) q) AS by_name,
  (SELECT round(avg(k)) FROM (SELECT * FROM a LEFT JOIN b USING (k)) q) AS by_star,
  (SELECT avg(k) FROM c) AS by_cte;
