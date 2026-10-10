-- …and at the other persisted door, whose predicate reaches the same walk.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
CREATE INDEX tix ON t (id) WHERE m IN ('ok', 'zzz');
