-- Composite UNIQUE (a, b) with default NULLS DISTINCT: a NULL in any key column
-- means the row is never a uniqueness candidate, so rows that agree on the
-- non-NULL column but carry a NULL in the other both insert (matches Postgres).
-- @setup
CREATE TABLE uq_null (id INT, a INT, b TEXT, UNIQUE (a, b));
INSERT INTO uq_null VALUES
  (1, 1, NULL),
  (2, 1, NULL),
  (3, NULL, 'x'),
  (4, NULL, 'x'),
  (5, 2, 'y');

-- @test
SELECT id, a, b FROM uq_null ORDER BY id;
