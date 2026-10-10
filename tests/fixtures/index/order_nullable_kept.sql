-- The NOT-NULL gate guard: an indexed NULLABLE column with a NULL row, ordered
-- `ASC`. Postgres `ASC` defaults to NULLS LAST, but the index orders NULLs FIRST
-- (the presence tag sorts before any present value) — so pushing an index-ordered
-- scan would misplace the NULL. pglet must KEEP the Sort here (pushdown skipped)
-- and return NULLS LAST, matching Postgres. The non-NULL values are unique and a
-- single NULL row keeps the order total and the positional compare deterministic.
-- @setup
CREATE TABLE ord_nullable (id INT PRIMARY KEY, n INT);
CREATE INDEX ord_nullable_n ON ord_nullable(n);
INSERT INTO ord_nullable VALUES
  (1, 30),
  (2, 10),
  (3, NULL),
  (4, 20);
-- @test
SELECT id, n FROM ord_nullable ORDER BY n ASC;
