-- date_trunc beside a string literal in a COALESCE and a CASE: the literal takes the
-- source's timestamp type, value and declaration.
-- @setup
CREATE TABLE dtf (id INT, ts TIMESTAMP, tz TIMESTAMPTZ);
INSERT INTO dtf VALUES (1, NULL, NULL), (2, '2020-01-02 03:04:05', '2020-01-02 03:04:05+00');
-- @test
SELECT id,
  coalesce(date_trunc('day', ts), '2021-05-05') AS c,
  CASE WHEN ts IS NULL THEN '2021-05-05' ELSE date_trunc('day', ts) END AS k,
  coalesce(date_trunc('month', tz), '2021-05-05 00:00:00+00') AS ctz
FROM dtf ORDER BY id;
