-- `u.*` over a data-modifying CTE relation (the spine's offer-supersede body,
-- `SELECT u.*, grantor.account_id …`). The wildcard expands against the CTE's
-- captured RETURNING columns, not the catalog.
-- @setup
CREATE TABLE accounts (id INT, name TEXT);
INSERT INTO accounts VALUES (1, 'Alice'), (2, 'Bob');
CREATE TABLE grants (id INT, owner INT, revoked BOOL);
INSERT INTO grants VALUES (10, 1, false), (11, 1, false), (12, 2, false);
-- @test
WITH updated AS (
  UPDATE grants SET revoked = true WHERE id = 12 RETURNING *
)
SELECT u.*, a.name
FROM updated u
JOIN accounts a ON a.id = u.owner;
