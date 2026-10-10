-- A `DELETE … RETURNING` data-modifying CTE joined to a base table.
-- @setup
CREATE TABLE accounts (id INT, name TEXT);
INSERT INTO accounts VALUES (1, 'Alice'), (2, 'Bob');
CREATE TABLE grants (id INT, owner INT, revoked BOOL);
INSERT INTO grants VALUES (10, 1, false), (11, 1, false), (12, 2, false);
-- @test
WITH removed AS (
  DELETE FROM grants WHERE owner = 1 RETURNING id, owner
)
SELECT u.id AS grant_id, a.name
FROM removed u
JOIN accounts a ON a.id = u.owner
ORDER BY u.id;
