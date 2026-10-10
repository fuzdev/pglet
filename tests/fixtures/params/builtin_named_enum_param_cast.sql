-- An undeclared `$1` cast to a builtin's name converts to the builtin, not to an enum
-- sharing the name.
-- @setup
CREATE TYPE int4 AS ENUM ('k');

-- @param $1 int4 5
-- @test
SELECT $1::int4 + 1 AS n;
