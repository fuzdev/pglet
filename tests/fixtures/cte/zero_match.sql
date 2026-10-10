-- A data-modifying CTE whose UPDATE matches no rows → an empty relation → an empty
-- join result (the spine's `if (revoked.length === 0)` early-out path).
-- @setup
CREATE TABLE accounts (id INT, name TEXT);
INSERT INTO accounts VALUES (1, 'Alice'), (2, 'Bob');
CREATE TABLE grants (id INT, owner INT, revoked BOOL);
INSERT INTO grants VALUES (10, 1, false);
-- @test
WITH updated AS (
  UPDATE grants SET revoked = true WHERE owner = 99 RETURNING id, owner
)
SELECT u.id AS grant_id, a.name
FROM updated u
JOIN accounts a ON a.id = u.owner
ORDER BY u.id;
