-- @setup
CREATE TABLE bool_test (id INT, active BOOL);
INSERT INTO bool_test VALUES (1, true);
INSERT INTO bool_test VALUES (2, false);
INSERT INTO bool_test VALUES (3, true);

-- @test
-- String 'true' should be coerced to BOOL when comparing to BOOL column
SELECT * FROM bool_test WHERE active = 'true' ORDER BY id;
