-- Rows leave and re-enter a float GIN index: an array naming NaN twice keys it once,
-- and its update and delete retract what was written.
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
UPDATE fkg SET arr = ARRAY[3.5]::float8[], ar4 = ARRAY[1]::real[] WHERE id = 4;
UPDATE fkg SET arr = '{NaN,NaN}' WHERE id = 1;
DELETE FROM fkg WHERE id = 2;
-- @test
SELECT 'nan' AS probe, id FROM fkg WHERE arr @> ARRAY['NaN'::float8]
UNION ALL
SELECT 'moved', id FROM fkg WHERE arr @> ARRAY[3.5::float8]
UNION ALL
SELECT 'zero', id FROM fkg WHERE arr @> ARRAY[0::float8]
ORDER BY 1, 2;
