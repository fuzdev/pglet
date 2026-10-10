-- The mirror of its sibling: the literal's conversion comes first in source order and
-- takes the statement instead. The pair is what makes the rule an interleaved walk.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK');

-- @test
-- @expect-error: zzz is not a member of mood, ahead of the later operator refusal
-- @expect-message
SELECT m > 'zzz' AND m > lower(s) FROM t;
