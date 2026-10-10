-- A join's column map is built from the input descriptors by its own code path
-- (`register_input_columns`), so the enum has to reach it there as well — an ordering
-- over the joined enum column is the shape that reads it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (id INT, tag TEXT);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1, 'a'), (2, 'b'), (3, 'c');

-- @test
SELECT t.id, u.tag, t.m::text AS label
FROM t JOIN u ON u.id = t.id
WHERE t.m > 'sad'
ORDER BY t.m;
