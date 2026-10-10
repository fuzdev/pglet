-- Over an indexed `real` column the list answers the same on both build profiles.
-- @profile-lock
-- @setup
CREATE TABLE inl_x (id INT PRIMARY KEY, x REAL);
CREATE INDEX inl_x_x ON inl_x (x);
INSERT INTO inl_x VALUES (1, 0.1), (2, 1.5), (3, 16777216), (4, NULL);
-- @test
SELECT id FROM inl_x WHERE x IN (0.1, 16777217) ORDER BY id;
