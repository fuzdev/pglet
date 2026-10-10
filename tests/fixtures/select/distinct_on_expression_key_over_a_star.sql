-- @setup
CREATE TABLE des (a INT, b INT, t TEXT);
INSERT INTO des VALUES (1, 5, 'x'), (1, 7, 'y'), (2, 5, 'z'), (3, 9, 'w');

-- The compound spelling of the same rule: `a+0` is not a whole bare name, so every
-- reference inside it resolves against the FROM relations — which the bare `*` projects
-- in full. Was `0A000` under a key rendered as `?column?`, naming nothing the user wrote.
-- @test
SELECT DISTINCT ON (a + 0) * FROM des ORDER BY a + 0, b DESC;
