-- The `ADD CONSTRAINT` route, over an empty table.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
ALTER TABLE t ADD CONSTRAINT ck CHECK (m <> 'q'::mood);
