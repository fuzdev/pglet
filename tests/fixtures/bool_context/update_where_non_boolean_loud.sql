-- The write half: the row was left untouched and the statement reported success.
-- @setup
CREATE TABLE bcuw (a INT);
INSERT INTO bcuw VALUES (9);
-- @expect-error: argument of WHERE must be type boolean
-- @test
UPDATE bcuw SET a = 1 WHERE a;
