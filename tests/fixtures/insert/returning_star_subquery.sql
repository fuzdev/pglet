-- `INSERT … RETURNING *, (correlated subquery) AS grant_count` — a non-sole `*`
-- in RETURNING plus a RETURNING-scoped scalar subquery against the just-inserted
-- row (the fuz_app cell_create shape). Bare `RETURNING *` already worked; the
-- mixed `*, <expr>` form and the subquery in RETURNING are what this covers.
-- @setup
CREATE TABLE cell (id INT PRIMARY KEY, name TEXT);
CREATE TABLE cell_grant (id INT, cell_id INT);
INSERT INTO cell_grant VALUES (10, 1), (11, 1);
-- @test
INSERT INTO cell (id, name) VALUES (1, 'a')
RETURNING *, (SELECT COUNT(*)::int FROM cell_grant WHERE cell_id = cell.id) AS grant_count;
