-- A long `OR` list stops at its first `true` operand wherever the division sits after it.
-- @setup
CREATE TABLE sl (a INT);
INSERT INTO sl VALUES (1);
-- @test
SELECT count(*) FROM sl WHERE a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a = 1 OR a/(a-1) = 1 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0 OR a < 0;
