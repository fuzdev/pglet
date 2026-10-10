-- A bound `text` parameter converted to `bytea` reaches the input function with no lexer
-- in front of it: the parameter's characters are what it reads.
-- @param $1 text abcd
-- @param $2 text \141\\
-- @param $3 text \x6162 63
-- @param $4 text ' '
-- @test
SELECT $1::text::bytea AS plain, $2::text::bytea AS escapes, $3::text::bytea AS hex, $4::text::bytea AS one_space;
