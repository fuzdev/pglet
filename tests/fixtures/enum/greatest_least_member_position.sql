-- `GREATEST`/`LEAST` reach their own comparator (`greatest_least`), which no other
-- shape in the corpus reaches — the bare literal is resolved to a member of the column's
-- enum, as 18.6 resolves it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, greatest(m, 'ok')::text AS g, least(m, 'ok')::text AS l FROM t ORDER BY id;
