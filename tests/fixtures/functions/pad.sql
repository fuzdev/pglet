-- lpad / rpad: pad or truncate to a character length; a multi-char fill wraps
-- cyclically; a shorter target truncates (keeping the leftmost chars); an empty
-- fill disables padding entirely.
-- @setup
CREATE TABLE pd (id INT, t TEXT);
INSERT INTO pd VALUES (1, 'hi'), (2, 'hello'), (3, NULL);
-- @test
SELECT id,
  lpad(t, 5, 'xy') AS lp,
  rpad(t, 5, 'xy') AS rp,
  lpad(t, 3) AS lp_trunc,
  lpad(t, 5, '') AS lp_empty,
  rpad(t, 5) AS rp_space
FROM pd ORDER BY id;
