-- substring: the SUBSTRING(x FROM n FOR m) / (x FROM n) / (x FOR m) special forms,
-- the comma form substring(x, n, m), and the negative-start clip
-- (`'12345' FROM -2 FOR 5` = '12' — the -2/-1 positions consume length but emit
-- nothing). 1-based, character-based.
-- @setup
CREATE TABLE sub (id INT, t TEXT);
INSERT INTO sub VALUES (1, 'Thomas'), (2, '12345'), (3, NULL);
-- @test
SELECT id,
  substring(t FROM 2 FOR 3) AS s1,
  substring(t FROM 2) AS s2,
  substring(t FOR 2) AS s3,
  substring(t, 2, 3) AS s4,
  substring('12345' FROM -2 FOR 5) AS gotcha
FROM sub ORDER BY id;
