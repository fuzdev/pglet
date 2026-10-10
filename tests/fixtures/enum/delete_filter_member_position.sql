-- The `DELETE`'s `WHERE`, the other write whose row set the comparison decides.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
DELETE FROM t WHERE m < 'ok';

-- @test
SELECT id FROM t ORDER BY id;
