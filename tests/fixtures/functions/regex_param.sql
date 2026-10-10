-- The pattern bound as a parameter, as kysely binds `'^pg_'`: the shape is judged on the
-- bound value.
-- @param $1 text ^pg_
-- @test
SELECT 'pg_x' ~ $1 AS matches, 'public' !~ $1 AS negated;
