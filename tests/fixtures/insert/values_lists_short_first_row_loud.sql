-- The length compared is the first row's, so a short **first** row refuses the longer row
-- after it: `42601`.
-- @setup
CREATE TABLE vlf (a INT, b INT DEFAULT 7);

-- @test
-- @expect-error: VALUES lists must all be the same length (42601)
-- @expect-message
INSERT INTO vlf VALUES (4), (5, 6) RETURNING a, b;
