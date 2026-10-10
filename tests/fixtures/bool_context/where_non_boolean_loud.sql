-- A non-boolean `WHERE` is `42804`, not a silent zero rows. The setup is
-- incapable of failing, so the assertion is the `@test`'s own.
-- @setup
CREATE TABLE bcn (a INT, b TEXT);
INSERT INTO bcn VALUES (9, 'x');
-- @expect-error: argument of WHERE must be type boolean
-- @test
SELECT count(*) FROM bcn WHERE a;
