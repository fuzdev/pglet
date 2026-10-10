-- The node positions take the same inference, off the same child enumeration the
-- gate reads.
-- @setup
CREATE TABLE bcp4 (a INT);
INSERT INTO bcp4 VALUES (1);
-- @param $1 bool true
-- @test
SELECT count(*) FROM bcp4 WHERE $1 AND a = 1;
