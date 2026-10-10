-- An `INSERT … RETURNING` data-modifying CTE joined to a base table.
-- @setup
CREATE TABLE accounts (id INT, name TEXT);
INSERT INTO accounts VALUES (1, 'Alice'), (2, 'Bob');
CREATE TABLE grants (id INT, owner INT, revoked BOOL);
INSERT INTO grants VALUES (10, 1, false);
-- @test
WITH added AS (
  INSERT INTO grants VALUES (20, 2, false), (21, 1, false) RETURNING id, owner
)
SELECT u.id AS grant_id, a.name
FROM added u
JOIN accounts a ON a.id = u.owner
ORDER BY u.id;
