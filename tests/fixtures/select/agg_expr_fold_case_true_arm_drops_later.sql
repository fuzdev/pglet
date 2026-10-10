-- After a `CASE` arm whose condition is a constant `true`, every later arm and the `ELSE`
-- are removed with their aggregates — also behind an earlier arm whose condition is not
-- constant.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE WHEN 1 = 1 THEN 0 ELSE sum(1/b) END AS t,
  CASE WHEN true THEN count(*) ELSE sum(1/b) END AS c,
  CASE WHEN count(*) > 5 THEN 1 WHEN true THEN 2 ELSE sum(1/b) END AS later
FROM u;
