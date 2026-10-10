-- A GIN index over `float8[]` decomposes each row into its elements and keys each as a
-- float: maintenance writes them and `@>` probes them.
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
SELECT id FROM fkg WHERE arr @> ARRAY[1.5]::float8[] ORDER BY id;
