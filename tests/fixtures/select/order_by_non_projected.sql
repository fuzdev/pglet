-- ORDER BY a column that is NOT in the SELECT list. Insertion order deliberately
-- differs from created_at order, so a planner that drops the non-projected sort
-- key (sorting the post-projection rows, which no longer carry created_at) would
-- silently return insertion order and diverge from Postgres.
-- @setup
CREATE TABLE events (id INT, created_at INT);
INSERT INTO events VALUES (1, 300), (2, 100), (3, 200);

-- @test
SELECT id FROM events ORDER BY created_at DESC;
