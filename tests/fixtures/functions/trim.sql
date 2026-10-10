-- trim family: the TRIM([BOTH|LEADING|TRAILING] [chars] FROM x) grammar plus the
-- ltrim/rtrim/btrim function forms. The trailing `chars` is a SET of characters
-- (membership), not a substring. Character-based.
-- @setup
CREATE TABLE tr (id INT, t TEXT);
INSERT INTO tr VALUES (1, '  hi  '), (2, 'xyxtrimyyx'), (3, NULL);
-- @test
SELECT id,
  trim(t) AS t_both,
  trim(leading from t) AS t_lead,
  trim(trailing from t) AS t_trail,
  trim('xy' FROM t) AS t_set,
  ltrim(t) AS lt,
  rtrim(t) AS rt,
  btrim(t, 'xy') AS bt
FROM tr ORDER BY id;
