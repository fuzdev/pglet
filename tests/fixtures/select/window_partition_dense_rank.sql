-- @setup
-- PARTITION BY with RANK and DENSE_RANK over ties (per-partition reset). The
-- ORDER BY tiebreaker `name` makes the row order total, so the output is
-- deterministic; the rank columns themselves exercise the tie behavior (RANK
-- leaves gaps, DENSE_RANK doesn't).
CREATE TABLE wsales (dept TEXT, name TEXT, amount INT);
INSERT INTO wsales VALUES
  ('eng', 'a', 100), ('eng', 'b', 100), ('eng', 'c', 90),
  ('sales', 'd', 50), ('sales', 'e', 40);

-- @test
SELECT dept, name, amount,
       RANK() OVER (PARTITION BY dept ORDER BY amount DESC) AS rnk,
       DENSE_RANK() OVER (PARTITION BY dept ORDER BY amount DESC) AS drnk
FROM wsales
ORDER BY dept, amount DESC, name;
