-- The plain side does not have to be a column either: `name` is in the string category
-- here as on 18.6, so a session source joins the `text` one on the far side of the pair.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for name and mood
SELECT current_user UNION ALL SELECT m FROM t;
