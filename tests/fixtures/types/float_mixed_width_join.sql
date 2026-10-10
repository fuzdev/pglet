-- A JOIN ON f8 = f4 matches across float widths with the total order: whole values and
-- NaN match, while a value like 0.1 that rounds differently at each width does not. So
-- rows 1 (1.5) and 3 (NaN) self-match but row 2 (0.1) does not — two matched pairs.
-- @setup
CREATE TABLE m (id INT, f8 float8, f4 float4);
INSERT INTO m VALUES (1, 1.5, 1.5), (2, 0.1, 0.1), (3, 'NaN', 'NaN');
-- @test
SELECT count(*) AS c FROM m a JOIN m b ON a.f8 = b.f4;
