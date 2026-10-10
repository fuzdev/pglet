-- @setup
CREATE TABLE sale (region TEXT, product TEXT, ts INT, amount INT);
INSERT INTO sale VALUES
  ('us', 'widget', 1, 10),
  ('us', 'widget', 2, 20),
  ('us', 'gadget', 1, 5),
  ('eu', 'widget', 3, 30),
  ('eu', 'widget', 1, 15);

-- @test
-- A two-column DISTINCT ON key: the latest (max ts) row per (region, product).
-- ON exprs lead the ORDER BY; the foreign `ts DESC` tiebreak picks the row per group.
SELECT DISTINCT ON (region, product) region, product, ts, amount
FROM sale
ORDER BY region, product, ts DESC;
