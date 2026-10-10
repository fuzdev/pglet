-- An index predicate holding the cast is refused at `CREATE INDEX`, over no row.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @profile-lock
-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
CREATE INDEX t_ix ON t (id) WHERE m <> 'q'::mood;
