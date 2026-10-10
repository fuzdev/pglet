-- The lower half of the same range read: `-infinity` is below `0001-01-01`.
-- @profile-lock
-- @setup
CREATE TABLE infpkb (d DATE PRIMARY KEY, v INT);
INSERT INTO infpkb VALUES ('infinity', 1), ('-infinity', 2), ('2000-01-01', 3), ('9999-12-31', 4), ('0001-01-01', 5);
-- @test
SELECT d, v FROM infpkb WHERE d <= date '0001-01-01' ORDER BY d DESC;
