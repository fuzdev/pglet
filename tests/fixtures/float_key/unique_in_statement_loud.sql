-- Two rows of one statement that differ only in zero's sign are duplicates of each
-- other.
-- @profile-lock
-- @expect-error: an in-statement duplicate (23505)
-- @expect-message
-- @setup
CREATE TABLE fku (n INT, x FLOAT8 UNIQUE);
-- @test
INSERT INTO fku VALUES (1, '0.0'::float8), (2, '-0.0'::float8);
