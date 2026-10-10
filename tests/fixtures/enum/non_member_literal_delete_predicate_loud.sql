-- The write polarity: the predicate answered `false` for every row here and takes the
-- whole statement there, so the rows agreed by accident. The `DELETE` below does not —
-- its `IN` list would have matched.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
DELETE FROM t WHERE m IN ('ok', 'zzz');
