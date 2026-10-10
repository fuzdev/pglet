-- A text-format parameter spelling `infinity`, cast to each type.
-- @param $1 text infinity
-- @param $2 text -infinity
-- @test
SELECT $1::text::date AS a, $2::text::timestamp AS b, $1::text::timestamptz AS c;
