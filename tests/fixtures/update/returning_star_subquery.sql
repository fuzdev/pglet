-- `UPDATE … RETURNING *, (correlated subquery) AS grant_count` — the bare-`*`
-- mixed RETURNING with a RETURNING-scoped scalar subquery (the fuz_app cell_update
-- shape; the qualified `o.*` form is covered by `from_returning_wildcard`).
-- @profile-lock
-- @setup
CREATE TABLE cell (id INT PRIMARY KEY, name TEXT);
CREATE TABLE cell_grant (id INT, cell_id INT);
INSERT INTO cell VALUES (1, 'a');
INSERT INTO cell_grant VALUES (10, 1), (11, 1), (12, 1);
-- @test
UPDATE cell SET name = 'z' WHERE id = 1
RETURNING *, (SELECT COUNT(*)::int FROM cell_grant WHERE cell_id = cell.id) AS grant_count;
