-- One constant beside a column of the left operand's own type is no array either: the
-- constant is compared at `float8`, so the stored `real` `0.1` matches only where `x = y`.
-- @setup
CREATE TABLE inl (id INT, x REAL, y REAL);
INSERT INTO inl VALUES (1, 0.1, 1.5), (2, 1.5, 0.3), (3, 0.3, 0.3), (4, NULL, NULL);
-- @test
SELECT id FROM inl WHERE x IN (0.1, y) ORDER BY id;
