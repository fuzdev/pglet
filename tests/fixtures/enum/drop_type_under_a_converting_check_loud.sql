-- A `CHECK` converting to the enum depends on it: `DROP TYPE` is `2BP01`, with the
-- constraint in the detail. It dropped the type and left a constraint naming it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mc (id INT, s TEXT CHECK (s::mood IS NOT NULL));

-- @test
-- @expect-error: the check depends on the type
-- @expect-message
DROP TYPE mood;
