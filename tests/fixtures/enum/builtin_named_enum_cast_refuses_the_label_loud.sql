-- The enum's own label is not an integer: with the builtin resolved first, `'k'::int4` is
-- the integer input's `22P02`, not a member of the same-named enum.
-- @setup
CREATE TYPE int4 AS ENUM ('k');

-- @expect-error: invalid input syntax for type integer
-- @test
SELECT 'k'::int4 AS a;
