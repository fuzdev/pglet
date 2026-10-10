-- The `SET DEFAULT` route of the cast.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
ALTER TABLE t ALTER COLUMN m SET DEFAULT 'q'::mood;
