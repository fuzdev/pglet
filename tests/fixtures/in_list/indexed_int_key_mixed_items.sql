-- A primary key beside a list whose common type is not the key's: `numeric` beside a numeric
-- constant, `real` beside a `real` item.
-- @profile-lock
-- @setup
CREATE TABLE inl_x (id INT PRIMARY KEY, x REAL);
INSERT INTO inl_x VALUES (1, 0.1), (2, 1.5), (3, 16777216);
-- @test
SELECT id, id IN (1, 2.0, 2.5) AS as_numeric, id IN ('1.0', 2.5) AS string_item, id IN (3, 2.5::real) AS as_real
FROM inl_x WHERE id IN (1, 2.0, 3.5) OR id IN (3, 2.5::real) ORDER BY id;
