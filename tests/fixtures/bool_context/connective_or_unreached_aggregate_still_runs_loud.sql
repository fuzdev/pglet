-- The `OR` and select-list spelling: the aggregate under the skipped operand still runs.
-- @setup
CREATE TABLE sa (id INT, g INT);
INSERT INTO sa VALUES (1, 1), (2, 1);
-- @expect-error: division by zero
-- @test
SELECT count(*) = 2 OR sum(1/(id-1)) > 0 FROM sa;
