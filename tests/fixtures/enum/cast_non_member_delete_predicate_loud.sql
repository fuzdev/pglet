-- It wrote: `'q'::mood > 'sad'::mood` was true, and the `DELETE` emptied the table where
-- 18.6 refuses the statement.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);
INSERT INTO t VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
DELETE FROM t WHERE 'q'::mood > 'sad'::mood;
