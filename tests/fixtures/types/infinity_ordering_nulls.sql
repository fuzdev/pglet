-- `-infinity` sorts before the domain's first day and `infinity` after its last, with NULL
-- after both ascending and before both descending.
-- @setup
CREATE TABLE inford (id INT, d DATE, ts TIMESTAMPTZ);
INSERT INTO inford VALUES (1, 'infinity', '-infinity'), (2, '0001-01-01', '9999-12-31 23:59:59.999999+00'),
  (3, NULL, NULL), (4, '-infinity', 'infinity'), (5, '9999-12-31', '0001-01-01 00:00:00+00');
-- @test
SELECT id, d, ts, row_number() OVER (ORDER BY d DESC) AS d_desc,
       row_number() OVER (ORDER BY ts NULLS FIRST) AS ts_nulls_first
FROM inford ORDER BY d, id;
