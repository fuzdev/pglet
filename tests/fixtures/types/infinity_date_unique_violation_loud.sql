-- Two `infinity` rows are one key.
-- @setup
CREATE TABLE infuq (d DATE PRIMARY KEY);
INSERT INTO infuq VALUES ('infinity'), ('-infinity');
-- @expect-error: duplicate key value violates unique constraint
-- @expect-message
-- @profile-lock
-- @test
INSERT INTO infuq VALUES ('infinity');
