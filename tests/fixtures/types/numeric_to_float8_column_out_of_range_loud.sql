-- The assignment of a numeric constant to a `float8` column is the same conversion.
-- @setup
CREATE TABLE fnd (d FLOAT8);
-- @test
-- @expect-error: out of range for type double precision (22003)
INSERT INTO fnd (d) VALUES (1e400);
