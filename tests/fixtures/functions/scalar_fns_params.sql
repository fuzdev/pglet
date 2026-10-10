-- Untyped parameters take the type 18.6 resolves the call to: text for the string
-- positions, int4 for left's count.
-- @param $1 text héllo
-- @param $2 int4 2
-- @test
SELECT left($1, $2) AS l, right($1, $2) AS r, md5($1) AS m, octet_length($1) AS o,
  substr($1, $2) AS s;
