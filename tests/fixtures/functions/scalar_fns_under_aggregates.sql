-- The new functions under the readers that type a computed operand — an aggregate's
-- argument, a fold's branch — answer 18.6's type and value there, not only at the top
-- of a select list.
-- @setup
CREATE TABLE ua (g INT, a INT, n NUMERIC, t TEXT, b BYTEA);
INSERT INTO ua VALUES
  (1, 3, 1.5, 'héllo', '\x0102'),
  (1, 4, 2.5, 'ab', '\x'),
  (2, 5, NULL, NULL, '\x01'),
  (2, NULL, 4, 'xyz', NULL);
-- @test
SELECT g,
  avg(mod(a, 2)) AS avg_mod,
  sum(mod(a, 2)) AS sum_mod,
  sum(mod(n, 2)) AS sum_nmod,
  round(avg(octet_length(t))) AS ravg_ol,
  sum(octet_length(b)) AS sum_ol,
  max(left(t, 1)) AS max_left,
  avg(power(a, 2)) AS avg_pow,
  max(substr(b, 1, 1)) AS max_sub
FROM ua GROUP BY g ORDER BY g;
