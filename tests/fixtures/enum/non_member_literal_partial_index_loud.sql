-- The second persisted-expression door: a partial index predicate. Recorded here and
-- refused there, `pg_indexes` empty on the far side of the restore.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
CREATE INDEX tix ON t (id) WHERE m > 'zzz';
