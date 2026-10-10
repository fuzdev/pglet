-- `m > 'sad'` compares the bare literal as the member 18.6 resolves it to, so it keeps
-- the members declared *after* `sad` — `(0 rows)` when the comparison is on the label
-- bytes, since every label sorts below `sad`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id FROM t WHERE m > 'sad' ORDER BY id;
