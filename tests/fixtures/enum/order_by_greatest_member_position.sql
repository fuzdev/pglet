-- `greatest`/`least` already ordered their own **arguments** by member position — the
-- value path resolves that from the argument list directly — while the *result* carried
-- no enum, so using the call as a sort key ordered by label bytes. Two derivations of
-- one fact, disagreeing: `greatest_least_member_position` pins the value half and was
-- green the whole time this half was wrong.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT id, greatest(m, 'sad')::text AS label FROM t ORDER BY greatest(m, 'sad'), id;
