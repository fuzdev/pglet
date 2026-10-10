-- A range read whose interval is empty — equal ends, both excluded — answers no row.
-- @profile-lock
-- @param $1 int4 2
-- @setup
CREATE TABLE ro (k INT PRIMARY KEY, v TEXT);
INSERT INTO ro VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
SELECT k FROM ro WHERE k > $1 AND k < $1;
