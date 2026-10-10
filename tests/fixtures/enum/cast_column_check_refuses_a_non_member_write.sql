-- A `CHECK` converting a column is accepted — its conversion is a value's — and runs on
-- every write: the row carrying `q` is refused.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mc (id INT, s TEXT CHECK (s::mood IS NOT NULL));
INSERT INTO mc VALUES (1, 'ok');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
INSERT INTO mc VALUES (2, 'q');
