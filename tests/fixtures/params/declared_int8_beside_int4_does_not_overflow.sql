-- A declared `bigint` `$N` beside an `integer` literal resolves over `bigint`, so a value
-- past `integer`'s range is kept rather than refused `22003` by a conversion to the
-- literal's type.
-- @param-declared $1 int8 5000000000
-- @test
SELECT coalesce($1, 1) AS c, least($1, 1) AS l, greatest($1, 1) AS g;
