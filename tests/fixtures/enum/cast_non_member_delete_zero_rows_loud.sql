-- The same refusal over a table with no row, where the predicate is never evaluated.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
DELETE FROM t WHERE m = 'q'::mood;
