-- greatest/least resolve a common type across mixed-width arguments: an int4/int8
-- mix widens to int8, int/numeric to numeric. The declared result type must match
-- the returned value's type (a binary-protocol client mis-decodes otherwise),
-- including when the wider-typed argument is NULL in a row.
-- @setup
CREATE TABLE gt (id INT, a INT, b BIGINT, n NUMERIC);
INSERT INTO gt VALUES (1, 1, 5, 2.5), (2, 9, NULL, 3), (3, NULL, 7, NULL);
-- @test
SELECT id,
  greatest(a, b) AS g_int,
  least(a, b) AS l_int,
  greatest(a, n) AS g_num,
  greatest(a::int4, 5::int8) AS g_lit
FROM gt ORDER BY id;
