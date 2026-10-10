-- The `JOIN … ON` spelling: the comparison sits in the join condition rather than in
-- `WHERE`, which is a clause of its own and a gate placement of its own.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL);
INSERT INTO w VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
CREATE TABLE u (id INT, m mood NOT NULL);
INSERT INTO u VALUES (1, 'ok');

-- @test
SELECT w.id FROM w JOIN (SELECT min(m) AS z FROM u) v ON w.m = v.z ORDER BY w.id;
