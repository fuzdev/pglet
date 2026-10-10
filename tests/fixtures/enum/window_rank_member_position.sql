-- A window `ORDER BY` sorts through the same shared comparator the `Sort` node does, but
-- reaches it by its own path (`compute_window_column`'s per-key placements), so `rank`
-- and `dense_rank` need the member order resolved there too.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy'), (4, 'ok');

-- @test
SELECT id, rank() OVER (ORDER BY m) AS r, dense_rank() OVER (ORDER BY m) AS d
FROM t ORDER BY id;
