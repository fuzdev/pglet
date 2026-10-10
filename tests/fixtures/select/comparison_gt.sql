-- @setup
CREATE TABLE nums (val INT);
INSERT INTO nums VALUES (10), (20), (30);

-- @test
SELECT * FROM nums WHERE val > 15 ORDER BY val;
