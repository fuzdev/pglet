-- An `IN` list element is an `=` against the subject, so a non-member element is refused
-- even where an earlier element would have matched.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT m IN ('ok', 'zzz') FROM t;
