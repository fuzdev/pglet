-- @setup
CREATE TABLE grants (actor_id INT, role TEXT, scope INT);
INSERT INTO grants VALUES
  (1, 'admin', 10), (1, 'admin', 20), (2, 'member', NULL), (2, 'member', NULL), (3, 'member', 10);

-- @test
-- `SELECT DISTINCT` dedups whole projected rows (two NULLs count as equal), the
-- spine's actor-search shape that collapses an actor holding several matching
-- grants. ORDER BY references the select-listed columns.
SELECT DISTINCT actor_id, role FROM grants ORDER BY actor_id, role;
