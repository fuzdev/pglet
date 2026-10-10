-- The consequence of key-order-sensitive equality that a user notices first:
-- `DISTINCT` and `GROUP BY` over a jsonb column split one logical value into two
-- groups. Three of these four rows are the *same* jsonb value written three ways.
--
-- Postgres folds them to one group; pglet reported three, silently — no error, just
-- a wrong row count in an aggregate over a document column.
-- @setup
CREATE TABLE jkog (id INT, d JSONB);
INSERT INTO jkog VALUES
  (1, '{"a":1,"b":2}'),
  (2, '{"b":2,"a":1}'),
  (3, '{"a":1,"b":2}'),
  (4, '{"a":9,"b":9}');
-- @test
SELECT d, count(*) AS n FROM jkog GROUP BY d ORDER BY d;
