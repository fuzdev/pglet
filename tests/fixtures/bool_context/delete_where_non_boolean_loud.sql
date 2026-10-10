-- @setup
CREATE TABLE bcdw (a INT);
INSERT INTO bcdw VALUES (9);
-- @expect-error: argument of WHERE must be type boolean
-- @test
DELETE FROM bcdw WHERE a;
