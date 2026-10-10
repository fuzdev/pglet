-- A bound `text` parameter holding an array literal, converted to `bytea[]`.
-- @param $1 text {abcd,\\x6162,"\\x6162","a b"}
-- @test
SELECT $1::text::bytea[] AS elements;
