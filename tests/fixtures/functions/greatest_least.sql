-- greatest / least skip NULL arguments and unify to a common type; all-NULL yields
-- NULL, so greatest(1, NULL, 3) = 3.
-- @setup
CREATE TABLE gl (id INT, a INT, b INT, c INT);
INSERT INTO gl VALUES (1, 1, 3, 2), (2, 1, NULL, 3), (3, NULL, NULL, NULL);
-- @test
SELECT id,
  greatest(a, b, c) AS g,
  least(a, b, c) AS l,
  greatest(a, NULL, c) AS gn
FROM gl ORDER BY id;
