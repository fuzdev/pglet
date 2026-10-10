-- A `CHECK` holding the cast is refused at `CREATE TABLE`, which holds no row.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
CREATE TABLE mc (id INT, m mood CHECK (m <> 'q'::mood));
