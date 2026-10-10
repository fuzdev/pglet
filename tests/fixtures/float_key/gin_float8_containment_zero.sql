-- `-0` and `0` are one element key.
-- @profile-lock
-- @setup
CREATE TABLE fkg (id INT, arr FLOAT8[], ar4 REAL[]);
CREATE INDEX fkg_arr ON fkg USING gin (arr);
CREATE INDEX fkg_ar4 ON fkg USING gin (ar4);
INSERT INTO fkg VALUES
  (1, ARRAY[1.5, 2.5], ARRAY[1.1, 2.5]::real[]),
  (2, '{-0,NaN}', '{NaN,NaN}'),
  (3, '{}', NULL),
  (4, '{NaN,NaN,0,-0,NULL}', ARRAY[0]::real[]);
-- @test
SELECT id FROM fkg WHERE arr @> ARRAY[0::float8] ORDER BY id;
