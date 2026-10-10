-- left / right: characters, not bytes (a multibyte string counts each character once);
-- a negative n is all but |n| from the other end; NULL in either argument is NULL; an
-- int2 count widens to the int4 overload; a string-literal count is resolved to int4.
-- `r_min` is the oracle minor's whole-string answer for right(t, -2147483648); upstream
-- back-patched a fix answering the empty string (commit a63df266), so this expectation
-- moves when the oracle moves to a minor release carrying it.
-- @setup
CREATE TABLE lr (id INT, t TEXT, v VARCHAR(10), n INT, s SMALLINT);
INSERT INTO lr VALUES
  (1, 'abcdef', 'abcdef', 2, 2),
  (2, 'héllo wörld', 'héllo', -3, -1),
  (3, 'abc', 'abc', -5, 0),
  (4, NULL, NULL, 1, 1),
  (5, 'abc', 'abc', NULL, NULL),
  (6, '', '', 3, 3);
-- @test
SELECT id,
  left(t, n) AS l, right(t, n) AS r,
  left(v, s) AS lv, right(v, s) AS rv,
  left(t, '2') AS l_unknown,
  right(t, 2147483647) AS r_max,
  left(t, -2147483648) AS l_min,
  right(t, -2147483648) AS r_min
FROM lr ORDER BY id;
