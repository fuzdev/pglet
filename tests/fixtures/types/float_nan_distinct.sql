-- SELECT DISTINCT folds the two NaN rows into one (NaN = NaN in Postgres's total
-- order) and ORDER BY sorts NaN last (greatest), yielding four rows 1, 2, 3, NaN.
-- @setup
CREATE TABLE nn (id INT, f float8);
INSERT INTO nn VALUES (1, 1.0), (2, 2.0), (3, 'NaN'), (4, 3.0), (5, 'NaN');
-- @test
SELECT DISTINCT f FROM nn ORDER BY f;
