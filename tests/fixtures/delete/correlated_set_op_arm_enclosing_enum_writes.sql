-- A `DELETE` whose predicate reads a set operation with an arm naming the target's enum
-- column removes the row 18.6 removes: the greatest of `{ok, sad}` and the row's own label
-- by member position, `> 'ok'` only for `happy` (`EDGES.md` row 159).
-- @setup
CREATE TYPE esw_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE esw_tc (a INT PRIMARY KEY, m esw_mood);
CREATE TABLE esw_jb (m esw_mood);
INSERT INTO esw_tc VALUES (1, 'sad'), (2, 'happy');
INSERT INTO esw_jb VALUES ('ok'), ('sad');
-- @test
DELETE FROM esw_tc
 WHERE (SELECT b.m FROM esw_jb b UNION SELECT esw_tc.m FROM esw_jb ORDER BY 1 DESC LIMIT 1) > 'ok'
RETURNING a;
