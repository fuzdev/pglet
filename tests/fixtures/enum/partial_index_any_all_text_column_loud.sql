-- The other persisted-expression position the same walk gates — an index predicate,
-- which decides membership and therefore the rows a unique index enforces over.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE w (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
-- @profile-lock
CREATE INDEX wix ON w (id) WHERE m = ANY(ARRAY[s]);
