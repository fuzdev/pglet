-- `SELECT *, (correlated subquery) AS grant_count` — a non-sole `*` projection
-- mixed with a computed column (the fuz_app cell list/get shape). The bare `*`
-- expands to the table's columns and the trailing item follows it.
-- @profile-lock
-- @setup
CREATE TABLE cell (id INT PRIMARY KEY, name TEXT);
CREATE TABLE cell_grant (id INT, cell_id INT);
INSERT INTO cell VALUES (1, 'a'), (2, 'b');
INSERT INTO cell_grant VALUES (10, 1), (11, 1), (12, 2);
-- @test
SELECT *, (SELECT COUNT(*)::int FROM cell_grant WHERE cell_id = cell.id) AS grant_count
FROM cell ORDER BY id;
