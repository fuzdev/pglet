-- Its negative control, bound the other way.
-- @setup
CREATE TABLE bcp2 (a INT);
INSERT INTO bcp2 VALUES (1);
-- @param $1 bool false
-- @test
SELECT count(*) FROM bcp2 WHERE $1;
