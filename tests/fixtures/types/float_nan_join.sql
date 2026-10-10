-- A JOIN ON a.f = b.f matches NaN to NaN (Postgres's NaN = NaN is true), so joining
-- two NaN rows against two NaN rows yields four matched pairs.
-- @setup
CREATE TABLE nn (id INT, f float8);
INSERT INTO nn VALUES (1, 1.0), (3, 'NaN'), (5, 'NaN');
-- @test
SELECT count(*) AS c FROM nn a JOIN nn b ON a.f = b.f WHERE a.f = 'NaN'::float8;
