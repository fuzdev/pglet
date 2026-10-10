-- `RETURNING` resolves its items through a relation-keyed namespace of its own
-- (`returning_column_map`), which is a third column-resolution object beside the
-- scan's and the join's — so the member order has to be threaded there too.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
INSERT INTO t VALUES (3, 'happy') RETURNING id, (m > 'ok') AS above_ok;
