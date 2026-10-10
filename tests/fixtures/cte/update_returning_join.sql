-- Data-modifying CTE: `WITH x AS (UPDATE … RETURNING …) SELECT … FROM x JOIN base`.
-- The spine's role_grant-revoke shape — the UPDATE's RETURNING rows become the
-- relation `u`, joined to an unrelated base table (`accounts`) to resolve a fan-out
-- column in one round-trip. The DML runs once; the body reads its captured rows.
-- @setup
CREATE TABLE accounts (id INT, name TEXT);
INSERT INTO accounts VALUES (1, 'Alice'), (2, 'Bob');
CREATE TABLE grants (id INT, owner INT, revoked BOOL);
INSERT INTO grants VALUES (10, 1, false), (11, 1, false), (12, 2, false);
-- @test
WITH updated AS (
  UPDATE grants SET revoked = true
  WHERE owner = 1 AND revoked = false
  RETURNING id, owner
)
SELECT u.id AS grant_id, a.name
FROM updated u
JOIN accounts a ON a.id = u.owner
ORDER BY u.id;
