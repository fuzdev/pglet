-- An unknown literal beside a computed `integer` branch adopts `integer`, as 18.6
-- resolves it: `COALESCE('7', length('ab'))` is the integer `7`, and so are its `CASE`,
-- `greatest`, `least` and `nullif` spellings; the `ARRAY` one is `integer[]`.
-- @test
SELECT COALESCE('7', length('ab')) AS c1, CASE WHEN true THEN '7' ELSE length('ab') END AS c2,
  ARRAY['7', length('ab')] AS c3, greatest('7', length('ab')) AS c4,
  least('7', length('ab')) AS c5, nullif('7', length('ab')) AS c6;
