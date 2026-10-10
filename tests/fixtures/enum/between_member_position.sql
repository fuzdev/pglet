-- `BETWEEN` desugars to `>= AND <=` in the parser, so it rides the same comparison —
-- listed separately because that desugaring is the only thing that makes it so, and a
-- fix that reached the operators through some other route would leave it behind.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id FROM t WHERE m BETWEEN 'ok' AND 'happy' ORDER BY id;
